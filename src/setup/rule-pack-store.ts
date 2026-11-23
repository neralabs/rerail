/**
 * Installing rule packs and finding the ones that fit a repository (docs/rule-packs.md, "Installing packs"). Packs
 * are npm packages, `rerail-pack-<id>` or `@<scope>/rerail-pack-<id>` (Rerail's own `@neralabs/rerail-pack-<id>`),
 * installed into the Rerail home's npm prefix (`$RERAIL_HOME/packs`), where the rule catalog reads them. An installed
 * pack is known by its package name, since two packages may hold the same pack id. Discovery fetches every pack core
 * lists into a staging folder of its own beside it, which installs nothing, checks each one as the catalog would, runs
 * its match definition against the repository, and removes the folder; only a person's answer installs.
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { join, resolve, sep } from "node:path";
import semver from "semver";
import { RULE_PACK_LIST } from "../engine/catalog/pack-list.generated.js";
import { removeScratch } from "../engine/file-replace.js";
import {
  activeCoreVersion,
  installedPackageName,
  installedPackDirectories,
  installedPacksPrefix,
  loadRuleCatalog,
  localSources,
  packDirectoriesOnDisk,
  readRuleCatalog,
} from "../engine/catalog/rule-catalog.js";
import type { RuleCatalog, SkippedRulePack } from "../engine/catalog/repository-model.js";
import { scanRepository, type RepositoryScan } from "../engine/catalog/rule-pack-matcher.js";
import {
  explainNoFit,
  packIdOfPackage,
  RULE_PACK_PACKAGE_NAME,
  rulePackPackageName,
  rulePackPackageOf,
  rulePacksThatFit,
  rulesStartingOn,
  type NoFitExplanation,
  type RulePack,
} from "../engine/catalog/rule-packs.js";
import { RERAIL_PACKAGE_NAME } from "../engine/package-root.js";
import { defaultRerailHome, UPDATE_STATE_FILE } from "../engine/storage/rerail-home.js";
import { CommandTimeoutError, outsideAnyProject, prefixInstallNpm, runNpm, unreachableRegistryReason, type NpmRunner } from "../runtime/npm.js";
import { formatDuration, NO_PROGRESS, type Progress } from "../runtime/progress.js";
import { withPacksLock } from "./packs-lock.js";

/** The dist-tag a pack is fetched under when core's update channel has no version of it: where every release publishes. */
export const PACK_FALLBACK_TAG = "next";
/** A staging folder older than this is left from a run that never finished, and is removed by the next fetch. */
const STALE_STAGING_MS = 60 * 60 * 1000;
/**
 * An npm install or uninstall in the packs prefix is stopped at this; a checked install then puts the prefix back. A
 * pack is a few MB (the largest of Rerail's, python, is about 6 MB unpacked): seconds on most lines. One stopped is
 * still well within the `PACKS_LOCK_WAIT_MS` another process waits for the packs lock.
 */
const PACK_INSTALL_TIMEOUT_MS = 2 * 60_000;

/** One pack at an exact version: what set-up shows a person and installs once they confirm. */
export interface PackVersion {
  /** The pack's npm package: `@neralabs/rerail-pack-<id>` for Rerail's own, else the third party's. */
  readonly packageName: string;
  /** The pack id the package holds. */
  readonly id: string;
  readonly version: string;
}

/** A pack fetched for discovery: its directory under the run's staging folder. */
export interface FetchedPack {
  readonly id: string;
  readonly directory: string;
}

export interface FetchProblem {
  readonly packageName: string;
  readonly reason: string;
}

/** What one fetch gave. Its directories stay until `release`, which removes them. */
export interface FetchedPacks {
  readonly fetched: readonly FetchedPack[];
  readonly problems: readonly FetchProblem[];
  release(): Promise<void>;
}

/** Where packs come from and go to: npm in production (`npmPackStore`); tests give their own. */
export interface PackStore {
  /**
   * The Rerail home whose packs it installs and removes. Discovery, installing and set-up read the installed packs
   * from here, so what they check is always what was installed.
   */
  readonly home: string;
  /** Each pack in `ids` at the version under `tag`, fetched without installing it; a pack that cannot be is a problem. */
  fetch(ids: readonly string[], tag: string): Promise<FetchedPacks>;
  /** Installs these npm specs (`<name>@<version>`, a tarball, a folder) into `home`'s packs. */
  install(specs: readonly string[]): Promise<void>;
  /** Uninstalls these packages from `home`'s packs. */
  remove(packageNames: readonly string[]): Promise<void>;
}

/**
 * The packs an answer's `--packs` names, each once: `<id>@<version>` for one of Rerail's own packs, or
 * `<package>@<version>` for any pack's package, as `rulePackShortName` names it. Throws on an entry that is neither,
 * at an exact version, or one pack named at two versions. Any id is taken, a project's own pack included: installing
 * it checks that npm has it, and that the catalog loads it.
 */
export function parsePackVersions(list: string): PackVersion[] {
  const packs = new Map<string, PackVersion>();
  for (const entry of list.split(",").map(item => item.trim()).filter(item => item !== "")) {
    const match = /^(.+)@(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/u.exec(entry);
    const packageName = match === null ? undefined : rulePackPackageOf(match[1]!);
    // The regex lets through what semver does not read (`01.2.3`, `1.0.0-a..b`); the downgrade guard compares with it.
    if (match === null || packageName === undefined || semver.valid(match[2]) === null) {
      throw new Error(`Name each rule pack as <id>@<version> or <package>@<version>, for example typescript@0.1.1, not ${entry}`);
    }
    const version = match[2]!;
    const named = packs.get(packageName);
    if (named !== undefined && named.version !== version) throw new Error(`${match[1]} is named twice, at ${named.version} and ${version}`);
    packs.set(packageName, { packageName, id: packIdOfPackage(packageName)!, version });
  }
  return [...packs.values()];
}

/** The npm spec that installs exactly `pack`. */
export const packSpec = (pack: PackVersion): string => `${pack.packageName}@${pack.version}`;

const NO_VERSION_UNDER_TAG = /\bETARGET\b|No matching version found|\bnotarget\b/u;
const NO_ACCESS = /\bE40[34]\b/u;

/**
 * Why npm would not give a pack, in one line. The packs are public, so a 404 means npm found no such package where it
 * looked: never published, or a registry or token in the npm configuration that refuses it. `runNpm` already words a
 * missing or expired login.
 */
export function fetchProblemReason(packageName: string, error: unknown): string {
  return timeoutReason(error) ?? npmFailureReason(packageName, error);
}

/**
 * Why a command `runCommand` stopped for running too long, in one line, with the last line it printed: npm's last
 * retry says what it was waiting for. Undefined for any other failure.
 */
function timeoutReason(error: unknown): string | undefined {
  if (!(error instanceof CommandTimeoutError)) return undefined;
  const last = error.output.split("\n").at(-1)!.trim();
  return `${error.command} did not finish within ${formatDuration(error.timeoutMs)} and was stopped${last === "" ? "" : ` (its last output: ${last})`}`;
}

/** Why an npm that ended on its own failed: no access, no registry, or its own `npm error` line. */
function npmFailureReason(packageName: string, error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  if (NO_ACCESS.test(message)) return `npm cannot read ${packageName}: it is not published, or the registry or token in ~/.npmrc refuses it`;
  const unreachable = unreachableRegistryReason(message);
  if (unreachable !== undefined) return unreachable;
  const lines = message.split("\n").map(line => line.trim()).filter(line => line !== "");
  const npmLine = lines.find(line => line.startsWith("npm error ") && !line.startsWith("npm error code"));
  return npmLine?.slice("npm error ".length) ?? lines[0] ?? "npm failed";
}

/** Removes the staging folders runs left that never finished; one in use is younger than `STALE_STAGING_MS`. */
async function removeStaleStaging(staging: string, now: number): Promise<void> {
  if (!existsSync(staging)) return;
  await Promise.all(readdirSync(staging).map(async entry => {
    const path = join(staging, entry);
    try {
      if (now - statSync(path).mtimeMs > STALE_STAGING_MS) await rm(path, { recursive: true, force: true });
    } catch {
      // Another run removed it first, or Windows still holds a file in it. One try is enough: the next fetch sweeps it
      // again, and waiting out a held folder here would hold up this fetch.
    }
  }));
}

/**
 * npm, in the Rerail home's packs prefix and a staging folder per fetch, so two discoveries at once never share one.
 * Install scripts never run, and `--` ends npm's options, so no spec is read as one.
 */
export function npmPackStore(home: string = defaultRerailHome(), runner: NpmRunner = runNpm): PackStore {
  const npm = prefixInstallNpm(runner);
  const prefix = installedPacksPrefix(home);
  const quiet = ["--ignore-scripts", "--omit=dev", "--no-audit", "--no-fund"];
  return {
    home,
    async fetch(ids, tag) {
      const staging = join(prefix, "staging");
      mkdirSync(staging, { recursive: true, mode: 0o700 });
      await removeStaleStaging(staging, Date.now());
      const run = await mkdtemp(join(staging, "run-"));
      const fetchOne = async (id: string): Promise<FetchedPack> => {
        const into = join(run, id);
        const install = (at: string) => npm(["install", "--prefix", into, ...quiet, "--no-package-lock", "--save-exact", "--", `${rulePackPackageName(id)}@${at}`]);
        try {
          await install(tag);
        } catch (error) {
          if (tag === PACK_FALLBACK_TAG || !(error instanceof Error) || !NO_VERSION_UNDER_TAG.test(error.message)) throw error;
          await install(PACK_FALLBACK_TAG);
        }
        return { id, directory: join(into, "node_modules", rulePackPackageName(id)) };
      };
      const settled = await Promise.allSettled(ids.map(fetchOne));
      const fetched: FetchedPack[] = [];
      const problems: FetchProblem[] = [];
      settled.forEach((result, index) => {
        const packageName = rulePackPackageName(ids[index]!);
        if (result.status === "fulfilled") fetched.push(result.value);
        else problems.push({ packageName, reason: fetchProblemReason(packageName, result.reason) });
      });
      // Discovery releases in a `finally`, so a removal that failed would replace its result or its error: the run's
      // folder is retried while Windows holds a file in it, then left for a later fetch's sweep.
      return { fetched, problems, release: () => removeScratch(run) };
    },
    async install(specs) {
      if (specs.length === 0) return;
      // Exact versions only: the tarballs discovery just fetched are in npm's cache under their integrity, so an
      // answer installs the version the person saw without asking the registry again. `rerail packs update` installs
      // a version its `npm view` just read, which writes the listing this install reads from the cache. A tag or
      // range must ask the registry. A version named by hand may be missing from an old cached listing (#992).
      const exact = specs.every(spec => semver.valid(spec.slice(spec.lastIndexOf("@") + 1)) !== null);
      await npm(["install", "--prefix", prefix, ...quiet, ...(exact ? ["--prefer-offline"] : []), "--save-exact", "--", ...specs], { timeoutMs: PACK_INSTALL_TIMEOUT_MS });
    },
    async remove(packageNames) {
      if (packageNames.length === 0) return;
      // npm reinstalls a listed package missing from node_modules while it uninstalls another: its scripts must not run.
      await npm(["uninstall", "--prefix", prefix, "--ignore-scripts", "--no-audit", "--no-fund", "--", ...packageNames], { timeoutMs: PACK_INSTALL_TIMEOUT_MS });
    },
  };
}

/** The update channel in the Rerail home's `update-state.json`: the dist-tag packs are fetched under. */
export function updateChannel(home: string = defaultRerailHome()): string {
  const path = join(home, UPDATE_STATE_FILE);
  if (!existsSync(path)) return "latest";
  return readChannel(path) ?? "latest";
}

/** The channel the update state file at `path` names; undefined when it names none or cannot be read. */
function readChannel(path: string): string | undefined {
  try {
    const state = JSON.parse(readFileSync(path, "utf8")) as unknown;
    if (typeof state === "object" && state !== null && "channel" in state && typeof state.channel === "string") return state.channel;
  } catch {
    // An unreadable state file is the updater's to report; discovery falls back to latest.
  }
  return undefined;
}

/** The version a pack directory's `package.json` names; null when it names none or cannot be read. */
function readPackageVersion(directory: string): string | null {
  try {
    const manifest = JSON.parse(readFileSync(join(directory, "package.json"), "utf8")) as unknown;
    return typeof manifest === "object" && manifest !== null && "version" in manifest && typeof manifest.version === "string" ? manifest.version : null;
  } catch {
    return null;
  }
}

/**
 * The version of package `packageName` installed in `home`'s packs, whether or not the catalog loads it; null when
 * none is.
 */
export function installedPackVersion(packageName: string, home: string = defaultRerailHome()): string | null {
  return readPackageVersion(join(installedPacksPrefix(home), "node_modules", packageName));
}

/** The installed packages in `home`'s packs that hold pack `id`, by package name: Rerail's own and any other. */
export function installedPackagesOf(id: string, home: string = defaultRerailHome()): string[] {
  return installedPackDirectories(home).map(installedPackageName).filter(name => packIdOfPackage(name) === id);
}

/** An installed pack with a newer version that fits the active core, and the dist-tag npm has it under. */
export interface PackUpdate {
  readonly packageName: string;
  readonly installed: string;
  readonly available: string;
  /** The update channel, or `next` for a pack the channel has no version of. */
  readonly tag: string;
}

/** What `newerPackVersions` found: the packs to update, and why a pack could not be checked. */
export interface PackUpdateCheck {
  readonly updates: readonly PackUpdate[];
  readonly problems: readonly string[];
}

/** Each `npm view` about a pack is cut short at this: the pack check runs after core's update is done. */
const PACK_CHECK_TIMEOUT_MS = 60_000;
/** npm's own limit on each request of a pack check, under `PACK_CHECK_TIMEOUT_MS`. */
const PACK_FETCH_TIMEOUT = "--fetch-timeout=30000";

/** A pack version npm has, with the core range it names; undefined when it names none. */
interface ViewedPack {
  readonly version: string;
  readonly coreRange: string | undefined;
}

/** What npm lists of a package: its dist-tags, and every version it has. */
interface PackListing {
  readonly distTags: ReadonlyMap<string, string>;
  readonly versions: readonly string[];
}

/** An installed pack `newerPackVersions` asks npm about. */
interface InstalledPack {
  readonly packageName: string;
  readonly version: string;
}

/** `value[key]` when `value` is an object holding `key`; undefined otherwise. */
function fieldOf(value: unknown, key: string): unknown {
  return typeof value === "object" && value !== null && key in value ? (value as Record<string, unknown>)[key] : undefined;
}

const stringOrUndefined = (value: unknown): string | undefined => typeof value === "string" ? value : undefined;

/** One version in what npm's `view <spec> name version peerDependencies --json` printed; undefined for none. */
function viewedPack(value: unknown): ViewedPack | undefined {
  const version = stringOrUndefined(fieldOf(value, "version"));
  if (version === undefined) return undefined;
  return { version, coreRange: stringOrUndefined(fieldOf(fieldOf(value, "peerDependencies"), RERAIL_PACKAGE_NAME)) };
}

/** Every version npm's `view <spec> name version peerDependencies --json` printed: an array when the spec matched more than one. */
function viewedPacks(stdout: string): ViewedPack[] {
  if (stdout.trim() === "") return [];
  const parsed = JSON.parse(stdout) as unknown;
  return (Array.isArray(parsed) ? parsed : [parsed]).flatMap(value => viewedPack(value) ?? []);
}

/** npm's `view <package> dist-tags versions --json`, read. */
function packListing(stdout: string): PackListing {
  const parsed = JSON.parse(stdout) as unknown;
  const tags = fieldOf(parsed, "dist-tags");
  const versions = fieldOf(parsed, "versions");
  return {
    distTags: new Map(Object.entries(typeof tags === "object" && tags !== null ? tags : {}).flatMap(([tag, version]) => typeof version === "string" ? [[tag, version] as const] : [])),
    versions: (Array.isArray(versions) ? versions : [versions]).filter((version): version is string => typeof version === "string"),
  };
}

/** The version under `tag` in `listing`, when it is one semver reads. */
function taggedVersion(listing: PackListing, tag: string): string | undefined {
  const version = listing.distTags.get(tag);
  return version !== undefined && semver.valid(version) !== null ? version : undefined;
}

/**
 * The tag a pack's newer versions are taken up to, with its version: the channel, or `next` when the channel has no
 * version of the pack, since set-up installs it from there then. Undefined when npm has neither.
 */
function packTag(listing: PackListing, channel: string): { readonly tag: string; readonly version: string } | undefined {
  for (const tag of [channel, PACK_FALLBACK_TAG]) {
    const version = taggedVersion(listing, tag);
    if (version !== undefined) return { tag, version };
  }
  return undefined;
}

/**
 * The versions in `listing` that may replace `installed`, newest first: newer than it and none newer than `tagged`, so
 * a pack never moves past its tag; a prerelease only when `tagged` is one, so a channel of releases gets none.
 */
function candidateVersions(listing: PackListing, tagged: string, installed: string): string[] {
  const prereleases = semver.prerelease(tagged) !== null;
  return listing.versions
    .filter(version => semver.valid(version) !== null && semver.gt(version, installed) && semver.lte(version, tagged)
      && (prereleases || semver.prerelease(version) === null))
    .sort(semver.rcompare);
}

/** Whether core `coreVersion` loads `pack` by its core range; one that names none fits any core. */
const fitsCore = (pack: ViewedPack, coreVersion: string): boolean =>
  pack.coreRange === undefined || semver.satisfies(coreVersion, pack.coreRange, { includePrerelease: true });

/**
 * The newest version of `pack` that core `coreVersion` loads, up to the version under `channel` (or `next`), when it is
 * newer than the installed one: one `npm view` for the listing, and one for the core ranges of every version that may
 * replace it, so a newest version made for a later core gives way to the newest one this core loads.
 */
async function newestFittingVersion(npm: NpmRunner, cwd: string, pack: InstalledPack, channel: string, coreVersion: string): Promise<PackUpdate | undefined> {
  const view = async (args: readonly string[]) => (await npm(["view", ...args, "--json", PACK_FETCH_TIMEOUT], { cwd, timeoutMs: PACK_CHECK_TIMEOUT_MS })).stdout;
  const listing = packListing(await view([pack.packageName, "dist-tags", "versions"]));
  const tagged = packTag(listing, channel);
  if (tagged === undefined) return undefined;
  const candidates = candidateVersions(listing, tagged.version, pack.version);
  if (candidates.length === 0) return undefined;
  // Each candidate exactly: npm reads `a || b` as a range and answers with every version it matches. It reduces every
  // answer to its one field when the first matched version has only one, losing the others' core ranges: `name`, which
  // every version has, keeps each answer whole.
  const answers = await view([`${pack.packageName}@${candidates.join(" || ")}`, "name", "version", "peerDependencies"]);
  const viewed = new Map(viewedPacks(answers).map(found => [found.version, found]));
  const available = candidates.find(version => viewed.has(version) && fitsCore(viewed.get(version)!, coreVersion));
  return available === undefined ? undefined : { packageName: pack.packageName, installed: pack.version, available, tag: tagged.tag };
}

/** The installed packs in `home` that npm has versions of: none from a folder or tarball, and none of a version semver cannot read. */
function npmInstalledPacks(home: string): InstalledPack[] {
  const recorded = recordedSpecs(home);
  return installedPackDirectories(home).flatMap((directory): InstalledPack[] => {
    const packageName = installedPackageName(directory);
    const version = readPackageVersion(directory);
    if (recorded.get(packageName)?.startsWith(`${packageName}@file:`) === true || version === null || semver.valid(version) === null) return [];
    return [{ packageName, version }];
  });
}

/**
 * The installed packs in `home` with a newer version that core `coreVersion` loads by its core range, each at the newest
 * such version up to the one under `channel`: what `rerail packs update` installs. A pack the channel has no version of
 * is taken up to its version under `next`, where set-up installs it from. A pack from a folder or tarball has no version
 * on npm to compare, and one installed newer than npm's under the tag (from `next`, by hand) is never offered an older
 * one. A pack npm has no version of under either tag is skipped; one npm could not be asked about is a problem, with why.
 */
export async function newerPackVersions(home: string, channel: string, coreVersion: string, npm: NpmRunner = runNpm): Promise<PackUpdateCheck> {
  const installed = npmInstalledPacks(home);
  if (installed.length === 0) return { updates: [], problems: [] };
  return await outsideAnyProject(async (cwd) => {
    const settled = await Promise.allSettled(installed.map(async pack => await newestFittingVersion(npm, cwd, pack, channel, coreVersion)));
    const updates: PackUpdate[] = [];
    const problems: string[] = [];
    settled.forEach((result, index) => {
      const pack = installed[index]!;
      if (result.status === "rejected") problems.push(`${pack.packageName}: ${fetchProblemReason(pack.packageName, result.reason)}`);
      else if (result.value !== undefined) updates.push(result.value);
    });
    return { updates, problems };
  });
}

/** A fetched pack, checked, with what matching it against the repository found. */
export interface DiscoveredPack {
  readonly pack: RulePack;
  /** The pack's npm package, Rerail's own or a third party's. */
  readonly packageName: string;
  readonly version: string;
  /** The rules on when the pack is added. */
  readonly ruleCount: number;
  /** What matched, and where the pack is added; undefined for a pack that does not fit the repository. */
  readonly match?: { readonly reason: string; readonly folder: string };
  /** The version installed now, if any: every repository on this computer runs it. */
  readonly installed: string | null;
}

export interface PackDiscovery {
  /** The packs that fit the repository, languages first: what set-up offers to install. */
  readonly recommended: readonly DiscoveredPack[];
  /** Every fetched pack that passed its checks and has a rule that starts on, recommended ones first. */
  readonly available: readonly DiscoveredPack[];
  /**
   * Fetched packs that passed their checks but have no rule that starts on (published before their rules pass the port
   * readiness gates), those that fit first: never offered, since adding one would run nothing, but named so a person
   * who expects one knows why it is missing.
   */
  readonly withoutRules: readonly DiscoveredPack[];
  /** Packs that could not be fetched or failed a check, and why. */
  readonly problems: readonly string[];
  /** When npm gave no pack at all, each distinct reason why: set-up cannot tell then which packs fit. */
  readonly downloadFailures?: readonly string[];
  /** The catalog of the fetched packs: what their rules would be, before any is installed. */
  readonly catalog: RuleCatalog;
  /** When the repository was scanned, packs were available, and none fits it: why not. */
  readonly nothingFits?: NoFitExplanation;
}

/** Why a pack was left out, naming its files from the pack's own folder: the package name already says which pack. */
const skippedReason = (skipped: SkippedRulePack): string => skipped.reason.replaceAll(`${skipped.path}${sep}`, "");

function skippedProblem(skipped: SkippedRulePack): string {
  const name = skipped.packageName === null ? skipped.path : `${skipped.packageName}${skipped.version === null ? "" : `@${skipped.version}`}`;
  return `${name}: ${skippedReason(skipped)}`;
}

/**
 * The installed packs in `home` that core `current` leaves out by their core range but `previous` loaded, each with
 * why: what `rerail update` names, since every repository that adds one stops running its rules.
 */
export function packsLeftOutByCore(home: string, current: string, previous: string | undefined): string[] {
  const directories = installedPackDirectories(home);
  if (directories.length === 0) return [];
  const leftOut = (coreVersion: string) => readRuleCatalog(directories, { coreVersion }).skipped;
  const before = new Set((previous === undefined ? [] : leftOut(previous)).map(skipped => skipped.path));
  return leftOut(current).filter(skipped => !before.has(skipped.path)).map(skippedProblem);
}

/**
 * Why no pack fits the scanned repository, explained over the packs with rules: only when none of the `discovered`
 * fits, since one that fits but has no rules yet is named with that reason, which is the whole answer.
 */
function nothingFits(scan: RepositoryScan | undefined, discovered: readonly DiscoveredPack[]): Pick<PackDiscovery, "nothingFits"> {
  const available = discovered.filter(found => found.ruleCount > 0);
  if (scan === undefined || available.length === 0 || discovered.some(found => found.match !== undefined)) return {};
  return { nothingFits: explainNoFit(scan, available.map(found => found.pack)) };
}

/** The catalog of the fetched packs in `fetched` and the installed ones in `standIns`, checked against the active core and the engine. */
function checkedCatalog(home: string, fetched: readonly FetchedPack[], standIns: readonly string[]): RuleCatalog {
  return readRuleCatalog([...fetched.map(pack => pack.directory), ...standIns], { coreVersion: activeCoreVersion(home), compile: true, localSources: localSources(home, standIns) });
}

/**
 * The catalog set-up offers packs from: the fetched packs `fromNpm`, and every other installed package standing in, a
 * third party's included (the catalog decides which of two holding one pack id loads, Rerail's own first). A fetched
 * version the catalog leaves out, made for a newer Rerail say, does not take the place of the version installed: that
 * one stands in for it, so the pack is still offered as it runs, and `leftOut` names the newer version with why. When
 * the version installed is the fetched one, it would be left out for the same reason, so the fetched one stays.
 */
function discoveryCatalog(home: string, fromNpm: readonly FetchedPack[]): { catalog: RuleCatalog; leftOut: readonly SkippedRulePack[] } {
  const installed = new Map(installedPackDirectories(home).map(directory => [installedPackageName(directory), directory]));
  const fetchedPackages = new Set(fromNpm.map(pack => rulePackPackageName(pack.id)));
  const standIns = [...installed].filter(([packageName]) => !fetchedPackages.has(packageName)).map(([, directory]) => directory);
  const catalog = checkedCatalog(home, fromNpm, standIns);
  const replaceable = (pack: FetchedPack): boolean => {
    const packageName = rulePackPackageName(pack.id);
    return installed.has(packageName) && installedPackVersion(packageName, home) !== readPackageVersion(pack.directory);
  };
  const leftOut = catalog.skipped.filter(skipped => fromNpm.some(pack => pack.directory === skipped.path && replaceable(pack)));
  if (leftOut.length === 0) return { catalog, leftOut };
  const kept = fromNpm.filter(pack => !leftOut.some(skipped => skipped.path === pack.directory));
  const keptPackages = new Set(kept.map(pack => rulePackPackageName(pack.id)));
  return { catalog: checkedCatalog(home, kept, [...installed].filter(([packageName]) => !keptPackages.has(packageName)).map(([, directory]) => directory)), leftOut };
}

/**
 * Every pack core lists, fetched under the update channel's tag, checked against the active core and the engine,
 * and matched against the repository at `repositoryRoot`; with none, nothing is read and no pack is recommended. The
 * installed version stands in for a pack npm cannot give (offline, no access, not published), and for one where the
 * version installed is newer than npm's under the tag (installed from `next` by hand), so set-up never offers to
 * replace a pack with an older one: when that newer version does not load, the pack is named with why, not offered.
 * It stands in too for a pack whose fetched version does not load (`discoveryCatalog`): the newer version is named
 * with why, and the installed one is still offered.
 * Installed packs are read from `store.home`. Discovery itself never fails on one pack.
 */
export async function discoverPacks(repositoryRoot: string | undefined, store: PackStore): Promise<PackDiscovery> {
  const { home } = store;
  const fetch = await store.fetch(RULE_PACK_LIST.map(pack => pack.id), updateChannel(home));
  try {
    // What is on disk, loaded or not: the version set-up would replace, and the one its downgrade guard compares.
    const newerInstalled = (fetched: FetchedPack): boolean => {
      const installed = installedPackVersion(rulePackPackageName(fetched.id), home);
      const offered = readPackageVersion(fetched.directory);
      return installed !== null && offered !== null && semver.valid(installed) !== null && semver.valid(offered) !== null && semver.gt(installed, offered);
    };
    const fromNpm = fetch.fetched.filter(fetched => !newerInstalled(fetched));
    const { catalog, leftOut } = discoveryCatalog(home, fromNpm);
    const packs = [...catalog.packs.values()];
    const scan = repositoryRoot === undefined ? undefined : scanRepository(repositoryRoot);
    const matched = new Map((scan === undefined ? [] : rulePacksThatFit(scan, packs, catalog.notRun)).map(found => [found.pack.id, found]));
    const discovered = packs.flatMap((pack): DiscoveredPack[] => {
      if (pack.version === null || pack.packageName === null) return [];
      const found = matched.get(pack.id);
      return [{
        pack,
        packageName: pack.packageName,
        version: pack.version,
        ruleCount: rulesStartingOn(pack, catalog.notRun),
        ...(found === undefined ? {} : { match: { reason: found.reason, folder: found.folder } }),
        installed: installedPackVersion(pack.packageName, home),
      }];
    });
    // Those that fit first, in catalog order within each.
    const ordered = [...discovered.filter(pack => pack.match !== undefined), ...discovered.filter(pack => pack.match === undefined)];
    const available = ordered.filter(pack => pack.ruleCount > 0);
    return {
      recommended: available.filter(pack => pack.match !== undefined),
      available,
      withoutRules: ordered.filter(pack => pack.ruleCount === 0),
      problems: [...fetch.problems.map(problem => `${problem.packageName}: ${problem.reason}`), ...leftOut.map(skippedProblem), ...catalog.skipped.map(skippedProblem)],
      ...(fetch.fetched.length === 0 && fetch.problems.length > 0 ? { downloadFailures: [...new Set(fetch.problems.map(problem => problem.reason))] } : {}),
      catalog,
      ...nothingFits(scan, discovered),
    };
  } finally {
    await fetch.release();
  }
}

/** One package in a home's packs prefix: the spec npm recorded for it, if any, and the version on disk, if any. */
interface PrefixEntry {
  readonly spec: string | undefined;
  readonly version: string | null;
}

/**
 * What npm recorded for each installed pack, by package name, whoever publishes it: the spec in the prefix's
 * `package.json` (`0.1.1`, or `file:…` for one added from a folder or tarball, made absolute), so a pack can be put
 * back exactly as it was installed.
 */
function recordedSpecs(home: string): Map<string, string> {
  const prefix = installedPacksPrefix(home);
  try {
    const manifest = JSON.parse(readFileSync(join(prefix, "package.json"), "utf8")) as { dependencies?: Record<string, unknown> };
    return new Map(Object.entries(manifest.dependencies ?? {}).flatMap(([name, spec]) => {
      if (!RULE_PACK_PACKAGE_NAME.test(name) || typeof spec !== "string") return [];
      return [[name, spec.startsWith("file:") ? `${name}@file:${resolve(prefix, spec.slice("file:".length))}` : `${name}@${spec}`] as const];
    }));
  } catch {
    return new Map();
  }
}

/**
 * Every pack package in `home`'s prefix, recorded or only on disk (a dependency npm hoisted there, which never loads),
 * with its spec and version, so putting the prefix back also takes out what an install brought along.
 */
function prefixState(home: string): Map<string, PrefixEntry> {
  const recorded = recordedSpecs(home);
  const names = new Set([...recorded.keys(), ...packDirectoriesOnDisk(home).map(installedPackageName)]);
  return new Map([...names].map(name => [name, { spec: recorded.get(name), version: installedPackVersion(name, home) }]));
}

/** The packages whose spec or version differs between two states of a prefix. */
function changedPacks(before: ReadonlyMap<string, PrefixEntry>, after: ReadonlyMap<string, PrefixEntry>): string[] {
  return [...new Set([...before.keys(), ...after.keys()])].filter(name => {
    const was = before.get(name);
    const now = after.get(name);
    return was?.spec !== now?.spec || (was?.version ?? null) !== (now?.version ?? null);
  });
}

/**
 * Puts `home`'s packs back as they were in `before`: each package whose spec or version changed is installed again
 * from the spec npm had recorded, or removed if it was not there. Each step runs even if the other fails. Returns what
 * to tell the person: nothing changed, or what failed and where to look.
 */
async function restorePrefix(store: PackStore, before: ReadonlyMap<string, PrefixEntry>): Promise<string> {
  const changed = changedPacks(before, prefixState(store.home));
  const restore = changed.flatMap(name => before.get(name)?.spec ?? []);
  const remove = changed.filter(name => before.get(name) === undefined);
  // Installed before with no record of how (copied into node_modules by hand): npm cannot put it back.
  const unrecorded = changed.filter(name => before.get(name) !== undefined && before.get(name)!.spec === undefined);
  const undone = unrecorded.map(name => `${name}@${before.get(name)!.version} was installed with no npm record to put it back from`);
  try {
    if (restore.length > 0) await store.install(restore);
  } catch (error) {
    undone.push(`putting back ${restore.join(", ")} failed: ${fetchProblemReason(restore.join(", "), error)}`);
  }
  try {
    if (remove.length > 0) await store.remove(remove);
  } catch (error) {
    const names = remove.join(", ");
    undone.push(`removing ${names} failed: ${fetchProblemReason(names, error)}`);
  }
  return undone.length === 0 ? "Nothing was changed." : `Then ${undone.join("; ")}. \`rerail packs list\` shows what is installed now.`;
}

/**
 * What a checked install did: the packs it installed or changed, each of which loads; or why not, with the prefix put
 * back as it was.
 */
export type PackInstall = { readonly installed: readonly PackVersion[] } | { readonly problem: string };

/**
 * One install into `store.home`'s packs, checked, holding the packs lock throughout so no other install changes the
 * prefix in between. `specsOf` gives what npm installs, from the prefix as it is; `expectedOf` the packs that must
 * then load at those versions, from the packages the install changed. Every pack the catalog loaded before must still
 * load from the same package (an install can push another out: a rule both hold, or a pack id Rerail's own pack
 * takes). When npm fails, or a pack is left out, the prefix is put back as it was (`restorePrefix`), and the problem
 * for the person says what is installed now.
 */
async function installChecked(
  store: PackStore,
  specsOf: (before: ReadonlyMap<string, PrefixEntry>) => readonly string[],
  expectedOf: (changed: readonly string[]) => readonly PackVersion[],
): Promise<PackInstall> {
  const { home } = store;
  return await withPacksLock(home, async () => {
    const loadedBefore = [...loadRuleCatalog(home).packs.values()];
    const before = prefixState(home);
    const specs = specsOf(before);
    if (specs.length > 0) {
      try {
        await store.install(specs);
      } catch (error) {
        // npm usually undoes a failed install itself; one that stopped partway (a full disk) may not have.
        return { problem: `${fetchProblemReason(specs.join(", "), error)}. ${await restorePrefix(store, before)}` };
      }
    }
    const expected = expectedOf(changedPacks(before, prefixState(home)));
    const catalog = loadRuleCatalog(home);
    const loads = (id: string, packageName: string | null, version: string | null): boolean =>
      catalog.packs.get(id)?.packageName === packageName && catalog.packs.get(id)?.version === version;
    const whyLeftOut = (packageName: string | null): string => {
      const skipped = catalog.skipped.find(entry => entry.packageName === packageName);
      return skipped === undefined ? "the catalog does not load it" : skippedReason(skipped);
    };
    const named = new Set(expected.map(pack => pack.id));
    const problems = [
      ...expected.filter(pack => !loads(pack.id, pack.packageName, pack.version)).map(pack => `${packSpec(pack)} cannot be used: ${whyLeftOut(pack.packageName)}`),
      ...loadedBefore.filter(pack => !named.has(pack.id) && !loads(pack.id, pack.packageName, pack.version))
        .map(pack => `${pack.packageName ?? pack.id} would stop loading: ${whyLeftOut(pack.packageName)}`),
    ];
    if (problems.length === 0) return { installed: expected };
    return { problem: `${problems.join("; ")}. ${await restorePrefix(store, before)}` };
  });
}

/** Set-up's install: exactly these versions, each of which must then load. Installs only those not installed already. */
export async function installPackVersions(packs: readonly PackVersion[], store: PackStore): Promise<string | undefined> {
  const result = await installChecked(
    store,
    before => packs.filter(pack => (before.get(pack.packageName)?.version ?? null) !== pack.version).map(packSpec),
    () => packs,
  );
  return "problem" in result ? result.problem : undefined;
}

/**
 * `rerail packs add`: these npm specs (a pack at a version, range or tag, a tarball, a folder), after which every pack
 * the install changed must load at the version it now has.
 */
export async function addPackSpecs(specs: readonly string[], store: PackStore): Promise<PackInstall> {
  return await installChecked(
    store,
    () => specs,
    changed => changed.flatMap(packageName => {
      const version = installedPackVersion(packageName, store.home);
      return version === null ? [] : [{ packageName, id: packIdOfPackage(packageName)!, version }];
    }),
  );
}

/** A pack `updatePacks` could not move, and why, which says what is installed now: mostly put back as it was. */
export interface FailedPackUpdate {
  readonly update: PackUpdate;
  readonly problem: string;
}

/** What `updatePacks` did: the packs it moved, those it could not, and why a pack could not be checked. */
export interface PackUpdateRun {
  readonly updated: readonly PackUpdate[];
  readonly failed: readonly FailedPackUpdate[];
  readonly problems: readonly string[];
}

/**
 * `rerail packs update`, which ends `rerail update`: moves each installed pack in `store.home` to the newest version
 * the active core loads (`newerPackVersions`, under the update channel). Each pack is installed and checked on its own
 * (`installChecked`), so one npm cannot install, or that does not load, is put back alone and the others still move. A
 * pack whose installed version changed since the check (another `rerail packs add` in between) is left as it is now.
 * An error that is no one pack's own (the packs lock held past its wait) ends the run there, reporting what moved first.
 */
export async function updatePacks(store: PackStore, progress: Progress = NO_PROGRESS, npm: NpmRunner = runNpm): Promise<PackUpdateRun> {
  const { home } = store;
  const check = await progress.step(
    "Checking your rule packs for newer versions",
    async () => await newerPackVersions(home, updateChannel(home), activeCoreVersion(home), npm),
    { afterMs: 30_000, hint: "npm may be slow or stuck; each npm request about a pack stops at 1 min." },
  );
  const updated: PackUpdate[] = [];
  const failed: FailedPackUpdate[] = [];
  for (const [index, update] of check.updates.entries()) {
    const pack: PackVersion = { packageName: update.packageName, id: packIdOfPackage(update.packageName)!, version: update.available };
    let result: PackInstall;
    try {
      result = await progress.step(`Installing ${packSpec(pack)}`, async () => await installChecked(
        store,
        before => before.get(update.packageName)?.version === update.installed ? [packSpec(pack)] : [],
        changed => changed.includes(update.packageName) ? [pack] : [],
      ), {
        afterMs: 30_000,
        hint: `npm may be slow or stuck, or another Rerail process is changing the rule packs; npm is stopped at ${formatDuration(PACK_INSTALL_TIMEOUT_MS)}.`,
      });
    } catch (error) {
      // Not a failure of this pack's own, which installChecked returns: the packs lock stayed held past its wait, or the
      // catalog could not be read, possibly after npm installed it. Every pack after it would meet the same, so none is
      // tried, and the packs already moved are still reported.
      const why = (error instanceof Error ? error.message : String(error)).replace(/\.+$/u, "");
      failed.push({ update, problem: `${why}. \`rerail packs list\` shows what is installed now.` });
      failed.push(...check.updates.slice(index + 1).map(next => ({ update: next, problem: `not tried, since updating ${update.packageName} stopped` })));
      break;
    }
    if ("problem" in result) failed.push({ update, problem: result.problem });
    else if (result.installed.length > 0) updated.push(update);
  }
  return { updated, failed, problems: check.problems };
}
