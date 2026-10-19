/**
 * `rerail packs`: the rule packs installed on this machine (docs/rule-packs.md, "Installing packs"). Packs are npm
 * packages installed into the Rerail home; a running daemon follows an install or removal without a restart.
 */
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import { parseArguments } from "../daemon/daemon.js";
import { repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { rulePackPackageName } from "../engine/catalog/rule-packs.js";
import { clean } from "../harness/setup-screens.js";
import { addPackSpecs, discoverPacks, npmPackStore, type DiscoveredPack, type PackStore } from "../setup/rule-pack-store.js";
import { withPacksLock } from "../setup/packs-lock.js";

const PACK_ID = /^[a-z][a-z0-9-]*$/u;
/**
 * A version (`0.1.1`, `0.1.2-next.0`), a range (`^0.1.0`) or a dist-tag (`next`): what may follow a pack's name. Not
 * npm's other specs, `npm:` aliases, git, URLs or `file:` paths, which would install something else under its name.
 */
const VERSION_OR_TAG = String.raw`[0-9A-Za-z^~<>=.*+-][0-9A-Za-z^~<>=.*+ -]*`;
const PACK_ID_AT_VERSION = new RegExp(String.raw`^([a-z][a-z0-9-]*)@(${VERSION_OR_TAG})$`, "u");
const PACK_PACKAGE = new RegExp(String.raw`^@neralabs\/rerail-pack-[a-z][a-z0-9-]*(?:@${VERSION_OR_TAG})?$`, "u");

/**
 * What npm installs for one `rerail packs add` argument: a pack id (`typescript`) or id at a version or tag
 * (`typescript@0.1.1`) is that pack's package; a pack's package name, at a version or tag, is itself; a tarball or
 * folder on disk is its absolute path. Anything else, an npm option (`--ignore-scripts=false`) or another kind of npm
 * spec, is refused, so the install keeps its own options and installs only a rule pack.
 */
export function packAddSpec(argument: string): string {
  if (argument.startsWith("-")) throw new Error(`rerail packs add takes rule packs, not npm options: ${argument}`);
  if (PACK_ID.test(argument)) return rulePackPackageName(argument);
  const atVersion = PACK_ID_AT_VERSION.exec(argument);
  if (atVersion !== null) return `${rulePackPackageName(atVersion[1]!)}@${atVersion[2]!}`;
  if (PACK_PACKAGE.test(argument)) return argument;
  // A folder or tarball that exists: npm is given its absolute path, never the argument, so it reads no other spec.
  const path = resolve(argument);
  if (existsSync(path)) return path;
  throw new Error(`${argument} is not a rule pack id, a @neralabs/rerail-pack-<id> package, or a tarball or folder that exists`);
}

function write(lines: readonly string[]): void {
  process.stdout.write(lines.length === 0 ? "" : `${lines.join("\n")}\n`);
}

/** The packs installed in `home` and those left out, as `rerail packs list` prints them. */
function installedLines(home: string): string[] {
  const catalog = loadRuleCatalog(home);
  const packs = [...catalog.packs.values()];
  const lines = packs.length === 0
    ? ["No rule packs are installed. `rerail packs recommend` lists the ones that fit a repository."]
    : packs.map(pack => `  ${pack.id}@${clean(pack.version ?? "?")}  ${clean(pack.name)}: ${pack.rules.filter(rule => rule.on).length} of ${pack.rules.length} rules start on`);
  for (const skipped of catalog.skipped) {
    lines.push(`  left out: ${clean(skipped.packageName ?? skipped.path)}${skipped.version === null ? "" : `@${clean(skipped.version)}`}: ${clean(skipped.reason)}`);
  }
  return lines;
}

/** One discovered pack; its name, reason and version come from the pack, so they are printed without control characters. */
function discoveredLine(found: DiscoveredPack): string {
  const installed = found.installed === null ? "" : found.installed === found.version ? "  (installed)" : `  (${clean(found.installed)} installed)`;
  // The reason names where each signal matched; the folder is where set-up would add the pack.
  const match = found.match === undefined ? "" : `: ${clean(found.match.reason)}${found.match.folder === "" ? "" : `; added in ${clean(found.match.folder)}/`}`;
  return `  ${rulePackPackageName(found.pack.id)}@${clean(found.version)} (${found.ruleCount} rules)${match}${installed}`;
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
    ...(discovery.problems.length === 0 ? [] : ["Not available:", ...discovery.problems.map(problem => `  ${clean(problem)}`)]),
    ...(discovery.recommended.length === 0 ? [] : [`Install them with: rerail packs add ${discovery.recommended.map(found => `${found.pack.id}@${found.version}`).join(" ")}`]),
  ]);
  return 0;
}

/** `rerail packs list|add|remove|recommend`, on `store`'s home. `add` and `remove` hold the packs lock while npm runs. */
export async function runPacksCommand(args: readonly string[], usage: () => string, store: PackStore = npmPackStore()): Promise<number> {
  const [subcommand, ...rest] = args;
  if (subcommand === "list" || subcommand === undefined) {
    parseArguments(rest, { flags: [], options: [] }, argument => `Unknown packs list option: ${argument}\n\n${usage()}`);
    write(installedLines(store.home));
    return 0;
  }
  if (subcommand === "add") {
    if (rest.length === 0) throw new Error(`Name a rule pack to add, for example rerail packs add typescript\n\n${usage()}`);
    // Checked as set-up's install is: a pack that installs but is left out is put back, and the command fails.
    const problem = await addPackSpecs(rest.map(packAddSpec), store);
    if (problem !== undefined) throw new Error(problem);
    write(installedLines(store.home));
    return 0;
  }
  if (subcommand === "remove") {
    const ids = rest.map(argument => argument.replace(/^@neralabs\/rerail-pack-/u, ""));
    const invalid = ids.find(id => !PACK_ID.test(id));
    if (ids.length === 0 || invalid !== undefined) throw new Error(`Name the rule packs to remove by id, for example rerail packs remove typescript\n\n${usage()}`);
    await withPacksLock(store.home, () => store.remove(ids));
    write(installedLines(store.home));
    return 0;
  }
  if (subcommand === "recommend") return await recommend(rest, usage, store);
  throw new Error(`Unknown packs command: ${subcommand}\n\n${usage()}`);
}
