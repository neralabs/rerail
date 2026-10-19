/**
 * `rerail packs`: the rule packs installed on this machine (docs/rule-packs.md, "Installing packs"). Packs are npm
 * packages installed into the Rerail home; a running daemon follows an install or removal without a restart.
 */
import { resolve } from "node:path";
import { parseArguments } from "../daemon/daemon.js";
import { repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { rulePackPackageName } from "../engine/catalog/rule-packs.js";
import { discoverPacks, npmPackStore, type DiscoveredPack, type PackStore } from "../setup/rule-pack-store.js";

const PACK_ID = /^[a-z][a-z0-9-]*$/u;
const PACK_ID_AT_VERSION = /^([a-z][a-z0-9-]*)@(\S+)$/u;

/**
 * What npm installs for one `rerail packs add` argument: a pack id (`typescript`) or id at a version or tag
 * (`typescript@0.1.1`) is that pack's package; anything else (a package name, a tarball, a folder) goes to npm as is.
 */
export function packAddSpec(argument: string): string {
  if (PACK_ID.test(argument)) return rulePackPackageName(argument);
  const atVersion = PACK_ID_AT_VERSION.exec(argument);
  return atVersion === null ? argument : `${rulePackPackageName(atVersion[1]!)}@${atVersion[2]!}`;
}

function write(lines: readonly string[]): void {
  process.stdout.write(lines.length === 0 ? "" : `${lines.join("\n")}\n`);
}

/** The installed packs and those left out, as `rerail packs list` prints them. */
function installedLines(): string[] {
  const catalog = loadRuleCatalog();
  const packs = [...catalog.packs.values()];
  const lines = packs.length === 0
    ? ["No rule packs are installed. `rerail packs recommend` lists the ones that fit a repository."]
    : packs.map(pack => `  ${pack.id}@${pack.version ?? "?"}  ${pack.name}: ${pack.rules.filter(rule => rule.on).length} of ${pack.rules.length} rules start on`);
  for (const skipped of catalog.skipped) {
    lines.push(`  left out: ${skipped.packageName ?? skipped.path}${skipped.version === null ? "" : `@${skipped.version}`}: ${skipped.reason}`);
  }
  return lines;
}

function discoveredLine(found: DiscoveredPack): string {
  const installed = found.installed === null ? "" : found.installed === found.version ? "  (installed)" : `  (${found.installed} installed)`;
  // The reason names where each signal matched; the folder is where set-up would add the pack.
  const match = found.match === undefined ? "" : `: ${found.match.reason}${found.match.folder === "" ? "" : `; added in ${found.match.folder}/`}`;
  return `  ${rulePackPackageName(found.pack.id)}@${found.version} (${found.ruleCount} rules)${match}${installed}`;
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
    ...(discovery.problems.length === 0 ? [] : ["Not available:", ...discovery.problems.map(problem => `  ${problem}`)]),
    ...(discovery.recommended.length === 0 ? [] : [`Install them with: rerail packs add ${discovery.recommended.map(found => `${found.pack.id}@${found.version}`).join(" ")}`]),
  ]);
  return 0;
}

/** `rerail packs list|add|remove|recommend`. */
export async function runPacksCommand(args: readonly string[], usage: () => string, store: PackStore = npmPackStore()): Promise<number> {
  const [subcommand, ...rest] = args;
  if (subcommand === "list" || subcommand === undefined) {
    parseArguments(rest, { flags: [], options: [] }, argument => `Unknown packs list option: ${argument}\n\n${usage()}`);
    write(installedLines());
    return 0;
  }
  if (subcommand === "add") {
    if (rest.length === 0) throw new Error(`Name a rule pack to add, for example rerail packs add typescript\n\n${usage()}`);
    await store.install(rest.map(packAddSpec));
    write(installedLines());
    return 0;
  }
  if (subcommand === "remove") {
    const ids = rest.map(argument => argument.replace(/^@neralabs\/rerail-pack-/u, ""));
    const invalid = ids.find(id => !PACK_ID.test(id));
    if (ids.length === 0 || invalid !== undefined) throw new Error(`Name the rule packs to remove by id, for example rerail packs remove typescript\n\n${usage()}`);
    await store.remove(ids);
    write(installedLines());
    return 0;
  }
  if (subcommand === "recommend") return await recommend(rest, usage, store);
  throw new Error(`Unknown packs command: ${subcommand}\n\n${usage()}`);
}
