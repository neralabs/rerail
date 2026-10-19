/**
 * Installing rule packs and finding the ones that fit a repository (docs/rule-packs.md, "Installing packs"). Packs
 * are npm packages, `@neralabs/rerail-pack-<id>`, installed into the Rerail home's npm prefix
 * (`$RERAIL_HOME/packs`), where the rule catalog reads them. Discovery fetches every pack core lists into a staging
 * prefix beside it, which installs nothing, checks each one as the catalog would, and runs its match definition
 * against the repository; only a person's answer installs.
 */
import { existsSync, readFileSync } from "node:fs";
import { basename, join, sep } from "node:path";
import { RULE_PACK_LIST } from "../engine/catalog/pack-list.generated.js";
import { activeCoreVersion, installedPackDirectories, installedPacksPrefix, loadRuleCatalog, readRuleCatalog } from "../engine/catalog/rule-catalog.js";
import type { RuleCatalog, SkippedRulePack } from "../engine/catalog/repository-model.js";
import { recommendRulePacks, rulePackPackageName, type RulePack } from "../engine/catalog/rule-packs.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import semver from "semver";
import { runNpm, type NpmRunner } from "../runtime/npm.js";

/** The dist-tag a pack is fetched under when core's update channel has no version of it: where packs publish first. */
export const PACK_FALLBACK_TAG = "next";

/** One pack at an exact version: what set-up shows a person and installs once they confirm. */
export interface PackVersion {
  readonly id: string;
  readonly version: string;
}

/** A pack fetched for discovery: its directory under the staging prefix. */
export interface FetchedPack {
  readonly id: string;
  readonly directory: string;
}

export interface FetchProblem {
  readonly packageName: string;
  readonly reason: string;
}

/** Where packs come from and go to: npm in production (`npmPackStore`); tests give their own. */
export interface PackStore {
  /** Each pack in `ids` at the version under `tag`, fetched without installing it; a pack that cannot be is a problem. */
  fetch(ids: readonly string[], tag: string): Promise<{ fetched: FetchedPack[]; problems: FetchProblem[] }>;
  /** Installs these npm specs (`<name>@<version>`, a tarball, a folder) into the Rerail home's packs. */
  install(specs: readonly string[]): Promise<void>;
  /** Uninstalls these packs from the Rerail home's packs. */
  remove(ids: readonly string[]): Promise<void>;
}

/** `typescript@0.1.1,complexity@0.1.1`, as an answer carries the packs a person confirmed. */
export function formatPackVersions(packs: readonly PackVersion[]): string {
  return packs.map(pack => `${pack.id}@${pack.version}`).join(",");
}

/** The packs an answer's `--packs` names; throws on an entry that is not `<id>@<version>`. */
export function parsePackVersions(list: string): PackVersion[] {
  return list.split(",").map(entry => entry.trim()).filter(entry => entry !== "").map(entry => {
    const match = /^([a-z][a-z0-9-]*)@(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/u.exec(entry);
    if (match === null) throw new Error(`Name each rule pack as <id>@<version>, for example typescript@0.1.1, not ${entry}`);
    return { id: match[1]!, version: match[2]! };
  });
}

/** The npm spec that installs exactly `pack`. */
export const packSpec = (pack: PackVersion): string => `${rulePackPackageName(pack.id)}@${pack.version}`;

const NO_VERSION_UNDER_TAG = /\bETARGET\b|No matching version found|\bnotarget\b/u;
const NO_ACCESS = /\bE40[34]\b/u;
const OFFLINE = /\b(ECONNREFUSED|ENOTFOUND|EAI_AGAIN|ETIMEDOUT|ECONNRESET|ENETUNREACH)\b/u;

/**
 * Why npm would not give a pack, in one line. npm answers a restricted package the person may not read with the same
 * 404 as one never published, so the two read alike. `runNpm` already words a missing or expired login.
 */
export function fetchProblemReason(packageName: string, error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (NO_ACCESS.test(message)) return `no read access to ${packageName}, or it is not published; ask Nera for access`;
  const offline = OFFLINE.exec(message);
  if (offline !== null) return `npm could not reach the registry (${offline[1]})`;
  const lines = message.split("\n").map(line => line.trim()).filter(line => line !== "");
  const npmLine = lines.find(line => line.startsWith("npm error ") && !line.startsWith("npm error code"));
  return npmLine?.slice("npm error ".length) ?? lines[0] ?? "npm failed";
}

/** npm, in the Rerail home's packs prefix and a staging prefix per pack. Install scripts never run. */
export function npmPackStore(home: string = defaultRerailHome(), npm: NpmRunner = runNpm): PackStore {
  const prefix = installedPacksPrefix(home);
  const quiet = ["--ignore-scripts", "--omit=dev", "--no-audit", "--no-fund"];
  const fetchOne = async (id: string, tag: string): Promise<FetchedPack> => {
    const staging = join(prefix, "staging", id);
    const install = (at: string) => npm(["install", "--prefix", staging, ...quiet, "--no-package-lock", "--save-exact", `${rulePackPackageName(id)}@${at}`]);
    try {
      await install(tag);
    } catch (error) {
      if (tag === PACK_FALLBACK_TAG || !(error instanceof Error) || !NO_VERSION_UNDER_TAG.test(error.message)) throw error;
      await install(PACK_FALLBACK_TAG);
    }
    return { id, directory: join(staging, "node_modules", "@neralabs", `rerail-pack-${id}`) };
  };
  return {
    async fetch(ids, tag) {
      const settled = await Promise.allSettled(ids.map(id => fetchOne(id, tag)));
      const fetched: FetchedPack[] = [];
      const problems: FetchProblem[] = [];
      settled.forEach((result, index) => {
        if (result.status === "fulfilled") fetched.push(result.value);
        else problems.push({ packageName: rulePackPackageName(ids[index]!), reason: fetchProblemReason(rulePackPackageName(ids[index]!), result.reason) });
      });
      return { fetched, problems };
    },
    async install(specs) {
      if (specs.length === 0) return;
      // Exact versions only: the tarballs discovery just fetched are in npm's cache under their integrity, so an
      // answer installs the version the person saw without asking the registry again. A tag or range must ask it.
      const exact = specs.every(spec => semver.valid(spec.slice(spec.lastIndexOf("@") + 1)) !== null);
      await npm(["install", "--prefix", prefix, ...quiet, ...(exact ? ["--prefer-offline"] : []), "--save-exact", ...specs]);
    },
    async remove(ids) {
      if (ids.length === 0) return;
      await npm(["uninstall", "--prefix", prefix, "--no-audit", "--no-fund", ...ids.map(rulePackPackageName)]);
    },
  };
}

/** The update channel in the Rerail home's `update-state.json`: the dist-tag packs are fetched under. */
export function updateChannel(home: string = defaultRerailHome()): string {
  const path = join(home, "update-state.json");
  if (!existsSync(path)) return "latest";
  try {
    const state = JSON.parse(readFileSync(path, "utf8")) as unknown;
    if (typeof state === "object" && state !== null && "channel" in state && typeof state.channel === "string") return state.channel;
  } catch {
    // An unreadable state file is the updater's to report; discovery falls back to latest.
  }
  return "latest";
}

/** A fetched pack, checked, with what matching it against the repository found. */
export interface DiscoveredPack {
  readonly pack: RulePack;
  readonly version: string;
  /** The rules on when the pack is added. */
  readonly ruleCount: number;
  /** What matched, and where the pack is added; undefined for a pack that does not fit the repository. */
  readonly match?: { readonly reason: string; readonly folder: string };
  /** The version installed now, if any. */
  readonly installed: string | null;
}

export interface PackDiscovery {
  /** The packs that fit the repository, languages first: what set-up offers to install. */
  readonly recommended: readonly DiscoveredPack[];
  /** Every fetched pack that passed its checks and has a rule that starts on, recommended ones first. */
  readonly available: readonly DiscoveredPack[];
  /** Packs that could not be fetched, or failed a check, and why. */
  readonly problems: readonly string[];
  /** The catalog of the fetched packs: what their rules would be, before any is installed. */
  readonly catalog: RuleCatalog;
}

function skippedProblem(skipped: SkippedRulePack): string {
  const name = skipped.packageName === null ? skipped.path : `${skipped.packageName}${skipped.version === null ? "" : `@${skipped.version}`}`;
  // A fetched pack's reasons name files under the staging prefix; the package name already says which pack.
  return `${name}: ${skipped.reason.replaceAll(`${skipped.path}${sep}`, "")}`;
}

/**
 * Every pack core lists, fetched under the update channel's tag, checked against the active core and the engine,
 * and matched against the repository at `repositoryRoot`; with none, nothing is read and no pack is recommended. A
 * pack npm cannot give (offline, no access, not published) is reported, and the installed version stands in for it
 * when there is one; discovery itself never fails on one.
 */
export async function discoverPacks(repositoryRoot: string | undefined, store: PackStore, home: string = defaultRerailHome()): Promise<PackDiscovery> {
  const { fetched, problems } = await store.fetch(RULE_PACK_LIST.map(pack => pack.id), updateChannel(home));
  const fetchedIds = new Set(fetched.map(pack => pack.id));
  const standIns = installedPackDirectories(home).filter(directory => !fetchedIds.has(basename(directory).slice("rerail-pack-".length)));
  const catalog = readRuleCatalog([...fetched.map(pack => pack.directory), ...standIns], { coreVersion: activeCoreVersion(home), compile: true });
  const installed = loadRuleCatalog().packs;
  const packs = [...catalog.packs.values()];
  const matched = new Map(repositoryRoot === undefined ? [] : recommendRulePacks(repositoryRoot, packs).map(found => [found.pack.id, found]));
  const discovered = packs.flatMap((pack): DiscoveredPack[] => {
    const ruleCount = pack.rules.filter(rule => rule.on).length;
    if (ruleCount === 0 || pack.version === null) return [];
    const found = matched.get(pack.id);
    return [{
      pack,
      version: pack.version,
      ruleCount,
      ...(found === undefined ? {} : { match: { reason: found.reason, folder: found.folder } }),
      installed: installed.get(pack.id)?.version ?? null,
    }];
  });
  const recommended = discovered.filter(pack => pack.match !== undefined);
  return {
    recommended,
    available: [...recommended, ...discovered.filter(pack => pack.match === undefined)],
    problems: [...problems.map(problem => `${problem.packageName}: ${problem.reason}`), ...catalog.skipped.map(skippedProblem)],
    catalog,
  };
}

/** Installs the packs whose installed version differs from the one named; the rest are installed already. */
export async function installPackVersions(packs: readonly PackVersion[], store: PackStore): Promise<void> {
  const installed = loadRuleCatalog().packs;
  await store.install(packs.filter(pack => installed.get(pack.id)?.version !== pack.version).map(packSpec));
}
