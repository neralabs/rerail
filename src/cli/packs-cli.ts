/**
 * `rerail packs`: the rule packs installed on this machine (docs/rule-packs.md, "Installing packs"). Packs are npm
 * packages installed into the Rerail home, Rerail's own (`@neralabs/rerail-pack-<id>`) or a third party's
 * (`rerail-pack-<id>`, `@<scope>/rerail-pack-<id>`); a running daemon follows an install or removal without a restart.
 */
import { existsSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { parseArguments } from "../daemon/daemon.js";
import { repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { isWorktreePack, WORKTREE_PACK_ID, worktreeRuleCatalog, worktreeRulesDirectory } from "../engine/catalog/worktree-pack.js";
import { openRerailDatabase, RERAIL_DATABASE_FILE } from "../engine/storage/database.js";
import { approvalKey, packSubject, readApprovals, ruleSubject, type RuleApprovalSubject } from "../engine/catalog/rule-approval.js";
import { ruleDocumentDigest } from "../engine/stable-json.js";
import { workspaceStartState } from "../runtime/repository-start.js";
import {
  packIdOfPackage,
  RERAIL_PUBLISHER,
  RULE_PACK_ID,
  RULE_PACK_PACKAGE_NAME,
  rulePackPackageName,
  rulePackPackageOf,
  rulePackPublisher,
  rulePackShortName,
  type RulePackLocalSource,
} from "../engine/catalog/rule-packs.js";
import { clean } from "../harness/setup-screens.js";
import {
  addPackSpecs,
  discoverPacks,
  installedPackagesOf,
  npmPackStore,
  type DiscoveredPack,
  type PackStore,
  type PackVersion,
} from "../setup/rule-pack-store.js";
import { withPacksLock } from "../setup/packs-lock.js";

/**
 * A version (`0.1.1`, `0.1.2-next.0`), a range (`^0.1.0`) or a dist-tag (`next`): what may follow a pack's name. Not
 * npm's other specs, `npm:` aliases, git, URLs or `file:` paths, which would install something else under its name.
 */
const VERSION_OR_TAG = String.raw`[0-9A-Za-z^~<>=.*+-][0-9A-Za-z^~<>=.*+ -]*`;
/** A pack id or package name (`rulePackPackageOf` tells which), then a version or tag if any. */
const PACK_AT_VERSION = new RegExp(String.raw`^((?:@[a-z0-9][a-z0-9._-]*\/)?[a-z][a-z0-9-]*)(?:@(${VERSION_OR_TAG}))?$`, "u");

/**
 * What npm installs for one `rerail packs add` argument: a pack id (`typescript`) or id at a version or tag
 * (`typescript@0.1.1`) is Rerail's own pack of that id; a pack's package name (`rerail-pack-<id>`,
 * `@<scope>/rerail-pack-<id>`), at a version or tag, is itself; a tarball or folder on disk is its absolute path.
 * Anything else, an npm option (`--ignore-scripts=false`) or another kind of npm spec, is refused, so the install keeps
 * its own options and installs only a rule pack.
 */
export function packAddSpec(argument: string): string {
  if (argument.startsWith("-")) throw new Error(`rerail packs add takes rule packs, not npm options: ${argument}`);
  const named = PACK_AT_VERSION.exec(argument);
  const packageName = named === null ? undefined : rulePackPackageOf(named[1]!);
  if (packageName !== undefined) return named![2] === undefined ? packageName : `${packageName}@${named![2]}`;
  // A folder or tarball that exists: npm is given its absolute path, never the argument, so it reads no other spec.
  const path = resolve(argument);
  if (existsSync(path)) return path;
  throw new Error(`${argument} is not a rule pack id, a rerail-pack-<id> or @<scope>/rerail-pack-<id> package, or a tarball or folder that exists`);
}

/**
 * The packages `rerail packs remove` uninstalls for one argument: a pack's package name is itself; a pack id is every
 * installed package that holds it, Rerail's own and any other, so no other pack takes the id once it is gone (with
 * none installed, Rerail's own package, which npm finds nothing to remove of). Undefined for anything else.
 */
function packagesToRemove(argument: string, home: string): string[] | undefined {
  if (RULE_PACK_PACKAGE_NAME.test(argument)) return [argument];
  if (!RULE_PACK_ID.test(argument)) return undefined;
  const installed = installedPackagesOf(argument, home);
  return installed.length === 0 ? [rulePackPackageName(argument)] : installed;
}

/**
 * Why removing `removed` would let another installed package take a pack id in every repository that adds it, or
 * undefined: a package removed by name that holds its id (it loads under it, or is Rerail's own, which keeps the id
 * while left out) while another package holding the same id stays.
 */
function takeoverProblem(removed: readonly string[], home: string): string | undefined {
  const catalog = loadRuleCatalog(home);
  const going = new Set(removed);
  for (const packageName of removed) {
    const id = packIdOfPackage(packageName)!;
    const holds = catalog.packs.get(id)?.packageName === packageName || rulePackPublisher(packageName) === RERAIL_PUBLISHER;
    const staying = installedPackagesOf(id, home).filter(name => !going.has(name));
    if (!holds || staying.length === 0) continue;
    return `Removing ${clean(packageName)} would let ${staying.map(clean).join(" or ")} run as pack ${id} in every repository that adds ${id}. `
      + `Remove them all with \`rerail packs remove ${id}\`, or remove ${staying.map(clean).join(" and ")} first.`;
  }
  return undefined;
}

/**
 * The warning `rerail packs add` prints for a pack it installed that is not one of Rerail's own, or that came from a
 * folder or tarball whatever its `package.json` calls it: nothing has vetted it, so its rules run in Log until a person
 * approves this version, and from then on what it says reaches the coding agent.
 */
function unreviewedWarning(pack: PackVersion, localSource: RulePackLocalSource | undefined): string {
  const what = localSource === undefined ? "is not one of Rerail's packs" : `was installed from the ${localSource.kind} ${clean(localSource.path)}, not from npm`;
  return `${clean(pack.packageName)}@${clean(pack.version)} ${what}, and Rerail has not reviewed it. `
    + "Its rules run in Log until a person reads them and approves this version "
    + `(rerail rules approve ${clean(pack.packageName)}); from then on, in every repository that adds it, their steer text reaches the coding agent.`;
}

function write(lines: readonly string[]): void {
  process.stdout.write(lines.length === 0 ? "" : `${lines.join("\n")}\n`);
}

/** The approvals recorded in `home`'s database, without creating it; none when there is no database yet. */
function approvalsIn(home: string): ReadonlySet<string> {
  const path = join(home, RERAIL_DATABASE_FILE);
  if (!existsSync(path)) return new Set();
  const database = openRerailDatabase(path, { readonly: true });
  try {
    return readApprovals(database);
  } finally {
    database.$client.close();
  }
}

/** Why `subject` steers no one yet, with what approves it, or "" when it needs no approval or has it. */
function waitingNote(subject: RuleApprovalSubject | undefined, name: string, approvals: ReadonlySet<string>): string {
  return subject === undefined || approvals.has(approvalKey(subject)) ? "" : `  waiting for approval, runs in Log until then: rerail rules approve ${clean(name)}`;
}

/**
 * The packs installed in `home` and those left out, as `rerail packs list` prints them. A pack installed from a folder
 * or tarball says so, with its path: its name is whatever that folder's `package.json` says, published by no one. A
 * pack waiting for a person's approval says so, with the command that approves it.
 */
function installedLines(home: string): string[] {
  const catalog = loadRuleCatalog(home);
  const packs = [...catalog.packs.values()];
  const approvals = approvalsIn(home);
  const lines = packs.length === 0
    ? ["No rule packs are installed. `rerail packs recommend` lists the ones that fit a repository."]
    : packs.flatMap(pack => {
      const name = pack.packageName === null ? pack.id : rulePackShortName(pack.packageName);
      const from = pack.localSource === undefined ? "" : `  (from the ${pack.localSource.kind} ${clean(pack.localSource.path)})`;
      const waiting = waitingNote(packSubject(catalog, pack), pack.packageName ?? pack.id, approvals);
      return [
        `  ${clean(name)}@${clean(pack.version ?? "?")}  ${clean(pack.name)}: ${pack.rules.filter(rule => rule.on).length} of ${pack.rules.length} rules start on${from}`,
        ...(waiting === "" ? [] : [`  ${waiting}`]),
      ];
    });
  for (const skipped of catalog.skipped) {
    lines.push(`  left out: ${clean(skipped.packageName ?? skipped.path)}${skipped.version === null ? "" : `@${clean(skipped.version)}`}: ${clean(skipped.reason)}`);
  }
  return lines;
}

/** Why the checkout at `checkoutRoot` runs no rules, in `home`'s database; undefined when it runs them. */
function notRunningLine(home: string, checkoutRoot: string): string | undefined {
  const state = workspaceStartState(checkoutRoot, join(home, RERAIL_DATABASE_FILE));
  if (state === "stopped") return "  Not running: Rerail is not on in this checkout. `rerail start` turns it on.";
  if (state === "not-set-up") return "  Not running: this repo is not set up yet, so no rule runs here. `rerail start` sets it up.";
  return undefined;
}

/**
 * The rules of the checkout at `checkoutRoot` in its own `.rerail/rules`, whether they run, and the documents there
 * left out, as `rerail packs list` prints them in a checkout; nothing without such rules.
 */
function worktreeLines(home: string, checkoutRoot: string): string[] {
  const installed = loadRuleCatalog(home);
  const catalog = worktreeRuleCatalog(installed, checkoutRoot);
  if (catalog === installed) return [];
  const pack = catalog.packs.get(WORKTREE_PACK_ID);
  const rules = pack !== undefined && isWorktreePack(pack) ? pack.rules : [];
  const directory = worktreeRulesDirectory(checkoutRoot);
  const notRunning = rules.length === 0 ? undefined : notRunningLine(home, checkoutRoot);
  const approvals = approvalsIn(home);
  return [
    `This worktree's own rules, in ${clean(directory)} (pack ${WORKTREE_PACK_ID}, here only): ${rules.length}`,
    ...(notRunning === undefined ? [] : [notRunning]),
    ...rules.map(rule => {
      const document = catalog.documents.get(rule.id);
      const waiting = waitingNote(document === undefined ? undefined : ruleSubject(rule.id, ruleDocumentDigest(document)), rule.id, approvals);
      return `  ${clean(rule.id)} (${rule.kind}, ${rule.mode})${waiting === "" ? "" : `\n  ${waiting}`}`;
    }),
    ...catalog.skipped.slice(installed.skipped.length)
      .map(skipped => `  left out: ${clean(relative(directory, skipped.path) || skipped.path)}: ${clean(skipped.reason)}`),
  ];
}

/** One discovered pack; its name, reason and version come from the pack, so they are printed without control characters. */
function discoveredLine(found: DiscoveredPack): string {
  const installed = found.installed === null ? "" : found.installed === found.version ? "  (installed)" : `  (${clean(found.installed)} installed)`;
  // The reason names where each signal matched; the folder is where set-up would add the pack.
  const match = found.match === undefined ? "" : `: ${clean(found.match.reason)}${found.match.folder === "" ? "" : `; added in ${clean(found.match.folder)}/`}`;
  return `  ${clean(found.packageName)}@${clean(found.version)} (${found.ruleCount} rules)${match}${installed}`;
}

async function recommend(args: readonly string[], usage: () => string, store: PackStore): Promise<number> {
  const { options } = parseArguments(args, { flags: [], options: ["--repo"] }, argument => `Unknown packs recommend option: ${argument}\n\n${usage()}`);
  const { workspaceRoot } = repositoryIdentityLocation(resolve(options.get("--repo") ?? process.cwd()));
  const discovery = await discoverPacks(workspaceRoot, store);
  const others = discovery.available.filter(found => found.match === undefined);
  write([
    discovery.recommended.length === 0 ? `No rule pack fits ${workspaceRoot}.` : `Rule packs that fit ${workspaceRoot}:`,
    ...discovery.recommended.map(discoveredLine),
    ...(others.length === 0 ? [] : ["Other rule packs:", ...others.map(discoveredLine)]),
    // Never offered, since adding one would run nothing; listed so a pack that fits is not simply missing.
    ...(discovery.withoutRules.length === 0 ? [] : ["No rules yet, so not offered:", ...discovery.withoutRules.map(discoveredLine)]),
    ...(discovery.problems.length === 0 ? [] : ["Not available:", ...discovery.problems.map(problem => `  ${clean(problem)}`)]),
    ...(discovery.recommended.length === 0 ? [] : [`Install them with: rerail packs add ${discovery.recommended.map(found => `${clean(rulePackShortName(found.packageName))}@${clean(found.version)}`).join(" ")}`]),
  ]);
  return 0;
}

/** `rerail packs list|add|remove|recommend`, on `store`'s home. `add` and `remove` hold the packs lock while npm runs. */
export async function runPacksCommand(args: readonly string[], usage: () => string, store: PackStore = npmPackStore()): Promise<number> {
  const [subcommand, ...rest] = args;
  if (subcommand === "list" || subcommand === undefined) {
    parseArguments(rest, { flags: [], options: [] }, argument => `Unknown packs list option: ${argument}\n\n${usage()}`);
    write([...installedLines(store.home), ...worktreeLines(store.home, repositoryIdentityLocation(process.cwd()).workspaceRoot)]);
    return 0;
  }
  if (subcommand === "add") {
    if (rest.length === 0) throw new Error(`Name a rule pack to add, for example rerail packs add typescript\n\n${usage()}`);
    // Checked as set-up's install is: a pack that installs but is left out is put back, and the command fails.
    const added = await addPackSpecs(rest.map(packAddSpec), store);
    if ("problem" in added) throw new Error(added.problem);
    const catalog = loadRuleCatalog(store.home);
    const warnings = added.installed.flatMap(pack => {
      const localSource = catalog.packs.get(pack.id)?.localSource;
      return rulePackPublisher(pack.packageName) === RERAIL_PUBLISHER && localSource === undefined ? [] : [unreviewedWarning(pack, localSource)];
    });
    write([...installedLines(store.home), ...warnings]);
    return 0;
  }
  if (subcommand === "remove") {
    if (rest.length === 0 || rest.some(argument => packagesToRemove(argument, store.home) === undefined)) {
      throw new Error(`Name the rule packs to remove by id or package, for example rerail packs remove typescript\n\n${usage()}`);
    }
    const removed = [...new Set(rest.flatMap(argument => packagesToRemove(argument, store.home)!))];
    await withPacksLock(store.home, async () => {
      // Read under the lock, so no install in between changes which package holds an id.
      const problem = takeoverProblem(removed, store.home);
      if (problem !== undefined) throw new Error(problem);
      await store.remove(removed);
    });
    write(installedLines(store.home));
    return 0;
  }
  if (subcommand === "recommend") return await recommend(rest, usage, store);
  throw new Error(`Unknown packs command: ${subcommand}\n\n${usage()}`);
}
