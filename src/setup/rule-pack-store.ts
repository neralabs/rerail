/**
 * Installing rule packs and finding the ones that fit a repository (docs/rule-packs.md, "Installing packs"). Packs
 * are npm packages, `@neralabs/rerail-pack-<id>`, installed into the Rerail home's npm prefix
 * (`$RERAIL_HOME/packs`), where the rule catalog reads them. Discovery fetches every pack core lists into a staging
 * folder of its own beside it, which installs nothing, checks each one as the catalog would, runs its match
 * definition against the repository, and removes the folder; only a person's answer installs.
 */
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { basename, join, resolve, sep } from "node:path";
import semver from "semver";
import { RULE_PACK_LIST } from "../engine/catalog/pack-list.generated.js";
import { activeCoreVersion, installedPackDirectories, installedPacksPrefix, loadRuleCatalog, readRuleCatalog } from "../engine/catalog/rule-catalog.js";
import type { RuleCatalog, SkippedRulePack } from "../engine/catalog/repository-model.js";
import { recommendRulePacks, rulePackPackageName, type RulePack } from "../engine/catalog/rule-packs.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { runNpm, type NpmRunner } from "../runtime/npm.js";
import { withPacksLock } from "./packs-lock.js";

/** The dist-tag a pack is fetched under when core's update channel has no version of it: where packs publish first. */
export const PACK_FALLBACK_TAG = "next";
/** A staging folder older than this is left from a run that never finished, and is removed by the next fetch. */
const STALE_STAGING_MS = 60 * 60 * 1000;

/** One pack at an exact version: what set-up shows a person and installs once they confirm. */
export interface PackVersion {
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
  /** Uninstalls these packs from `home`'s packs. */
  remove(ids: readonly string[]): Promise<void>;
}

/** `typescript@0.1.1,complexity@0.1.1`, as an answer carries the packs a person confirmed. */
export function formatPackVersions(packs: readonly PackVersion[]): string {
  return packs.map(pack => `${pack.id}@${pack.version}`).join(",");
}

/**
 * The packs an answer's `--packs` names, each once. Throws on an entry that is not `<id>@<version>` (an exact
 * version), or one pack named at two versions. Any id is taken, a project's own pack included: installing it checks
 * that npm has it, and that the catalog loads it.
 */
export function parsePackVersions(list: string): PackVersion[] {
  const packs = new Map<string, PackVersion>();
  for (const entry of list.split(",").map(item => item.trim()).filter(item => item !== "")) {
    const match = /^([a-z][a-z0-9-]*)@(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)$/u.exec(entry);
    // The regex lets through what semver does not read (`01.2.3`, `1.0.0-a..b`); the downgrade guard compares with it.
    if (match === null || semver.valid(match[2]) === null) throw new Error(`Name each rule pack as <id>@<version>, for example typescript@0.1.1, not ${entry}`);
    const [, id, version] = match as unknown as [string, string, string];
    const named = packs.get(id);
    if (named !== undefined && named.version !== version) throw new Error(`${id} is named twice, at ${named.version} and ${version}`);
    packs.set(id, { id, version });
  }
  return [...packs.values()];
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

/** Removes the staging folders runs left that never finished; one in use is younger than `STALE_STAGING_MS`. */
async function removeStaleStaging(staging: string, now: number): Promise<void> {
  if (!existsSync(staging)) return;
  await Promise.all(readdirSync(staging).map(async entry => {
    const path = join(staging, entry);
    try {
      if (now - statSync(path).mtimeMs > STALE_STAGING_MS) await rm(path, { recursive: true, force: true });
    } catch {
      // Another run removed it first.
    }
  }));
}

/**
 * npm, in the Rerail home's packs prefix and a staging folder per fetch, so two discoveries at once never share one.
 * Install scripts never run, and `--` ends npm's options, so no spec is read as one.
 */
export function npmPackStore(home: string = defaultRerailHome(), npm: NpmRunner = runNpm): PackStore {
  const prefix = installedPacksPrefix(home);
  const quiet = ["--ignore-scripts", "--omit=dev", "--no-audit", "--no-fund"];
  return {
    home,
    async fetch(ids, tag) {
      const staging = join(prefix, "staging");
      mkdirSync(staging, { recursive: true });
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
        return { id, directory: join(into, "node_modules", "@neralabs", `rerail-pack-${id}`) };
      };
      const settled = await Promise.allSettled(ids.map(fetchOne));
      const fetched: FetchedPack[] = [];
      const problems: FetchProblem[] = [];
      settled.forEach((result, index) => {
        const packageName = rulePackPackageName(ids[index]!);
        if (result.status === "fulfilled") fetched.push(result.value);
        else problems.push({ packageName, reason: fetchProblemReason(packageName, result.reason) });
      });
      return { fetched, problems, release: () => rm(run, { recursive: true, force: true }) };
    },
    async install(specs) {
      if (specs.length === 0) return;
      // Exact versions only: the tarballs discovery just fetched are in npm's cache under their integrity, so an
      // answer installs the version the person saw without asking the registry again. A tag or range must ask it.
      const exact = specs.every(spec => semver.valid(spec.slice(spec.lastIndexOf("@") + 1)) !== null);
      await npm(["install", "--prefix", prefix, ...quiet, ...(exact ? ["--prefer-offline"] : []), "--save-exact", "--", ...specs]);
    },
    async remove(ids) {
      if (ids.length === 0) return;
      // npm reinstalls a listed package missing from node_modules while it uninstalls another: its scripts must not run.
      await npm(["uninstall", "--prefix", prefix, "--ignore-scripts", "--no-audit", "--no-fund", "--", ...ids.map(rulePackPackageName)]);
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

/** The version a pack directory's `package.json` names; null when it names none or cannot be read. */
function readPackageVersion(directory: string): string | null {
  try {
    const manifest = JSON.parse(readFileSync(join(directory, "package.json"), "utf8")) as unknown;
    return typeof manifest === "object" && manifest !== null && "version" in manifest && typeof manifest.version === "string" ? manifest.version : null;
  } catch {
    return null;
  }
}

/** The version of pack `id` installed in `home`'s packs, whether or not the catalog loads it; null when none is. */
export function installedPackVersion(id: string, home: string = defaultRerailHome()): string | null {
  return readPackageVersion(join(installedPacksPrefix(home), "node_modules", "@neralabs", `rerail-pack-${id}`));
}

/** A fetched pack, checked, with what matching it against the repository found. */
export interface DiscoveredPack {
  readonly pack: RulePack;
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
  /** Packs that could not be fetched or failed a check, and why. */
  readonly problems: readonly string[];
  /** The catalog of the fetched packs: what their rules would be, before any is installed. */
  readonly catalog: RuleCatalog;
}

/** Why a pack was left out, naming its files from the pack's own folder: the package name already says which pack. */
const skippedReason = (skipped: SkippedRulePack): string => skipped.reason.replaceAll(`${skipped.path}${sep}`, "");

function skippedProblem(skipped: SkippedRulePack): string {
  const name = skipped.packageName === null ? skipped.path : `${skipped.packageName}${skipped.version === null ? "" : `@${skipped.version}`}`;
  return `${name}: ${skippedReason(skipped)}`;
}

/**
 * The installed packs in `home` that core `current` leaves out by their core range but `previous` loaded, each with
 * why: what `rerail update` and `update rollback` name, since every repository that adds one stops running its rules.
 */
export function packsLeftOutByCore(home: string, current: string, previous: string | undefined): string[] {
  const directories = installedPackDirectories(home);
  if (directories.length === 0) return [];
  const leftOut = (coreVersion: string) => readRuleCatalog(directories, { coreVersion }).skipped;
  const before = new Set((previous === undefined ? [] : leftOut(previous)).map(skipped => skipped.path));
  return leftOut(current).filter(skipped => !before.has(skipped.path)).map(skippedProblem);
}

/** The pack id an installed or staged pack directory holds (`…/@neralabs/rerail-pack-<id>`). */
const packIdOf = (directory: string): string => basename(directory).slice("rerail-pack-".length);

/**
 * Every pack core lists, fetched under the update channel's tag, checked against the active core and the engine,
 * and matched against the repository at `repositoryRoot`; with none, nothing is read and no pack is recommended. The
 * installed version stands in for a pack npm cannot give (offline, no access, not published), and for one where the
 * version installed is newer than npm's under the tag (installed from `next` by hand), so set-up never offers to
 * replace a pack with an older one: when that newer version does not load, the pack is named with why, not offered.
 * Installed packs are read from `store.home`. Discovery itself never fails on one pack.
 */
export async function discoverPacks(repositoryRoot: string | undefined, store: PackStore): Promise<PackDiscovery> {
  const { home } = store;
  const fetch = await store.fetch(RULE_PACK_LIST.map(pack => pack.id), updateChannel(home));
  try {
    // What is on disk, loaded or not: the version set-up would replace, and the one its downgrade guard compares.
    const newerInstalled = (fetched: FetchedPack): boolean => {
      const installed = installedPackVersion(fetched.id, home);
      const offered = readPackageVersion(fetched.directory);
      return installed !== null && offered !== null && semver.valid(installed) !== null && semver.valid(offered) !== null && semver.gt(installed, offered);
    };
    const fromNpm = fetch.fetched.filter(fetched => !newerInstalled(fetched));
    const fetchedIds = new Set(fromNpm.map(pack => pack.id));
    const standIns = installedPackDirectories(home).filter(directory => !fetchedIds.has(packIdOf(directory)));
    const catalog = readRuleCatalog([...fromNpm.map(pack => pack.directory), ...standIns], { coreVersion: activeCoreVersion(home), compile: true });
    const packs = [...catalog.packs.values()];
    const matched = new Map((repositoryRoot === undefined ? [] : recommendRulePacks(repositoryRoot, packs)).map(found => [found.pack.id, found]));
    const discovered = packs.flatMap((pack): DiscoveredPack[] => {
      const ruleCount = pack.rules.filter(rule => rule.on).length;
      if (ruleCount === 0 || pack.version === null) return [];
      const found = matched.get(pack.id);
      return [{
        pack,
        version: pack.version,
        ruleCount,
        ...(found === undefined ? {} : { match: { reason: found.reason, folder: found.folder } }),
        installed: installedPackVersion(pack.id, home),
      }];
    });
    const recommended = discovered.filter(pack => pack.match !== undefined);
    return {
      recommended,
      available: [...recommended, ...discovered.filter(pack => pack.match === undefined)],
      problems: [...fetch.problems.map(problem => `${problem.packageName}: ${problem.reason}`), ...catalog.skipped.map(skippedProblem)],
      catalog,
    };
  } finally {
    await fetch.release();
  }
}

/** One pack in a home's packs prefix: the spec npm recorded for it, if any, and the version on disk, if any. */
interface PrefixEntry {
  readonly spec: string | undefined;
  readonly version: string | null;
}

/**
 * What npm recorded for each installed pack, by id: the spec in the prefix's `package.json` (`0.1.1`, or `file:…` for
 * one added from a folder or tarball, made absolute), so a pack can be put back exactly as it was installed.
 */
function recordedSpecs(home: string): Map<string, string> {
  const prefix = installedPacksPrefix(home);
  try {
    const manifest = JSON.parse(readFileSync(join(prefix, "package.json"), "utf8")) as { dependencies?: Record<string, unknown> };
    return new Map(Object.entries(manifest.dependencies ?? {}).flatMap(([name, spec]) => {
      if (!name.startsWith("@neralabs/rerail-pack-") || typeof spec !== "string") return [];
      const id = name.slice("@neralabs/rerail-pack-".length);
      return [[id, spec.startsWith("file:") ? `${name}@file:${resolve(prefix, spec.slice("file:".length))}` : `${name}@${spec}`] as const];
    }));
  } catch {
    return new Map();
  }
}

/** Every pack in `home`'s prefix, recorded or on disk, with its spec and version. */
function prefixState(home: string): Map<string, PrefixEntry> {
  const recorded = recordedSpecs(home);
  const ids = new Set([...recorded.keys(), ...installedPackDirectories(home).map(packIdOf)]);
  return new Map([...ids].map(id => [id, { spec: recorded.get(id), version: installedPackVersion(id, home) }]));
}

/** The packs whose spec or version differs between two states of a prefix. */
function changedPacks(before: ReadonlyMap<string, PrefixEntry>, after: ReadonlyMap<string, PrefixEntry>): string[] {
  return [...new Set([...before.keys(), ...after.keys()])].filter(id => {
    const was = before.get(id);
    const now = after.get(id);
    return was?.spec !== now?.spec || (was?.version ?? null) !== (now?.version ?? null);
  });
}

/**
 * Puts `home`'s packs back as they were in `before`: each pack whose spec or version changed is installed again from
 * the spec npm had recorded, or removed if it was not there. Each step runs even if the other fails. Returns what to
 * tell the person: nothing changed, or what failed and where to look.
 */
async function restorePrefix(store: PackStore, before: ReadonlyMap<string, PrefixEntry>): Promise<string> {
  const changed = changedPacks(before, prefixState(store.home));
  const restore = changed.flatMap(id => before.get(id)?.spec ?? []);
  const remove = changed.filter(id => before.get(id) === undefined);
  // Installed before with no record of how (copied into node_modules by hand): npm cannot put it back.
  const unrecorded = changed.filter(id => before.get(id) !== undefined && before.get(id)!.spec === undefined);
  const undone = unrecorded.map(id => `${rulePackPackageName(id)}@${before.get(id)!.version} was installed with no npm record to put it back from`);
  try {
    if (restore.length > 0) await store.install(restore);
  } catch (error) {
    undone.push(`putting back ${restore.join(", ")} failed: ${fetchProblemReason(restore.join(", "), error)}`);
  }
  try {
    if (remove.length > 0) await store.remove(remove);
  } catch (error) {
    const names = remove.map(rulePackPackageName).join(", ");
    undone.push(`removing ${names} failed: ${fetchProblemReason(names, error)}`);
  }
  return undone.length === 0 ? "Nothing was changed." : `Then ${undone.join("; ")}. \`rerail packs list\` shows what is installed now.`;
}

/**
 * One install into `store.home`'s packs, checked, holding the packs lock throughout so no other install changes the
 * prefix in between. `specsOf` gives what npm installs, from the prefix as it is; `expectedOf` the packs that must
 * then load at those versions, from the packs the install changed. Every pack the catalog loaded before must still
 * load (an install can push another out: a rule both hold). When npm fails, or a pack is left out, the prefix is put
 * back as it was (`restorePrefix`). Returns the problem for the person, saying what is installed now; undefined when
 * every pack loads.
 */
async function installChecked(
  store: PackStore,
  specsOf: (before: ReadonlyMap<string, PrefixEntry>) => readonly string[],
  expectedOf: (changed: readonly string[]) => readonly PackVersion[],
): Promise<string | undefined> {
  const { home } = store;
  return await withPacksLock(home, async () => {
    const loadedBefore = new Map([...loadRuleCatalog(home).packs.values()].map(pack => [pack.id, pack.version]));
    const before = prefixState(home);
    const specs = specsOf(before);
    if (specs.length > 0) {
      try {
        await store.install(specs);
      } catch (error) {
        // npm usually undoes a failed install itself; one that stopped partway (a full disk) may not have.
        return `${fetchProblemReason(specs.join(", "), error)}. ${await restorePrefix(store, before)}`;
      }
    }
    const expected = expectedOf(changedPacks(before, prefixState(home)));
    const catalog = loadRuleCatalog(home);
    const whyLeftOut = (id: string): string => {
      const skipped = catalog.skipped.find(entry => entry.packageName === rulePackPackageName(id));
      return skipped === undefined ? "the catalog does not load it" : skippedReason(skipped);
    };
    const named = new Set(expected.map(pack => pack.id));
    const problems = [
      ...expected.filter(pack => catalog.packs.get(pack.id)?.version !== pack.version).map(pack => `${packSpec(pack)} cannot be used: ${whyLeftOut(pack.id)}`),
      ...[...loadedBefore].filter(([id, version]) => !named.has(id) && catalog.packs.get(id)?.version !== version)
        .map(([id]) => `${rulePackPackageName(id)} would stop loading: ${whyLeftOut(id)}`),
    ];
    if (problems.length === 0) return undefined;
    return `${problems.join("; ")}. ${await restorePrefix(store, before)}`;
  });
}

/** Set-up's install: exactly these versions, each of which must then load. Installs only those not installed already. */
export async function installPackVersions(packs: readonly PackVersion[], store: PackStore): Promise<string | undefined> {
  return await installChecked(
    store,
    before => packs.filter(pack => (before.get(pack.id)?.version ?? null) !== pack.version).map(packSpec),
    () => packs,
  );
}

/**
 * `rerail packs add`: these npm specs (a pack at a version, range or tag, a tarball, a folder), after which every pack
 * the install changed must load at the version it now has.
 */
export async function addPackSpecs(specs: readonly string[], store: PackStore): Promise<string | undefined> {
  return await installChecked(
    store,
    () => specs,
    changed => changed.flatMap(id => {
      const version = installedPackVersion(id, store.home);
      return version === null ? [] : [{ id, version }];
    }),
  );
}
