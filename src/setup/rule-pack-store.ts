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
import {
  packIdOfPackage,
  RULE_PACK_PACKAGE_NAME,
  rulePackPackageName,
  rulePackPackageOf,
  rulePacksThatFit,
  type RulePack,
} from "../engine/catalog/rule-packs.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { runNpm, unreachableRegistryReason, type NpmRunner } from "../runtime/npm.js";
import { withPacksLock } from "./packs-lock.js";

/** The dist-tag a pack is fetched under when core's update channel has no version of it: where a held pack publishes. */
export const PACK_FALLBACK_TAG = "next";
/** A staging folder older than this is left from a run that never finished, and is removed by the next fetch. */
const STALE_STAGING_MS = 60 * 60 * 1000;

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
    async remove(packageNames) {
      if (packageNames.length === 0) return;
      // npm reinstalls a listed package missing from node_modules while it uninstalls another: its scripts must not run.
      await npm(["uninstall", "--prefix", prefix, "--ignore-scripts", "--no-audit", "--no-fund", "--", ...packageNames]);
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
      const installed = installedPackVersion(rulePackPackageName(fetched.id), home);
      const offered = readPackageVersion(fetched.directory);
      return installed !== null && offered !== null && semver.valid(installed) !== null && semver.valid(offered) !== null && semver.gt(installed, offered);
    };
    const fromNpm = fetch.fetched.filter(fetched => !newerInstalled(fetched));
    // Every other installed package stands in, a third party's included: the catalog decides which of two holding
    // one pack id loads, Rerail's own first.
    const fetchedPackages = new Set(fromNpm.map(pack => rulePackPackageName(pack.id)));
    const standIns = installedPackDirectories(home).filter(directory => !fetchedPackages.has(installedPackageName(directory)));
    const catalog = readRuleCatalog([...fromNpm.map(pack => pack.directory), ...standIns], {
      coreVersion: activeCoreVersion(home), compile: true, localSources: localSources(home, standIns),
    });
    const packs = [...catalog.packs.values()];
    const matched = new Map((repositoryRoot === undefined ? [] : rulePacksThatFit(repositoryRoot, packs)).map(found => [found.pack.id, found]));
    const discovered = packs.flatMap((pack): DiscoveredPack[] => {
      if (pack.version === null || pack.packageName === null) return [];
      const found = matched.get(pack.id);
      return [{
        pack,
        packageName: pack.packageName,
        version: pack.version,
        ruleCount: pack.rules.filter(rule => rule.on).length,
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
      problems: [...fetch.problems.map(problem => `${problem.packageName}: ${problem.reason}`), ...catalog.skipped.map(skippedProblem)],
      ...(fetch.fetched.length === 0 && fetch.problems.length > 0 ? { downloadFailures: [...new Set(fetch.problems.map(problem => problem.reason))] } : {}),
      catalog,
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
