/**
 * The rule catalog: every rule and pack Rerail can run, read from the packs installed in the Rerail home
 * (`$RERAIL_HOME/packs/node_modules`: `rerail-pack-<id>` and `@<scope>/rerail-pack-<id>`, Rerail's own under
 * `@neralabs`). Core ships no packs. The installed files are the rules (docs/rule-packs.md, "Where a pack's rules
 * live"): nothing copies them into the database, which names packs and rules by id. Each pack is checked on its own,
 * and one that fails a check is reported in `skipped` and left out while the others load.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import semver from "semver";
import { CodeRuleCompileError, EngineUnavailableError, storableCodeRule } from "../code-rule/code-rule-compile.js";
import { PACKAGE_ROOT, RERAIL_PACKAGE_NAME } from "../package-root.js";
import { compareDeterministicStrings, ruleDocumentDigest, stableJsonText } from "../stable-json.js";
import { defaultRerailHome } from "../storage/rerail-home.js";
import type { RuleCatalog, SkippedRulePack } from "./repository-model.js";
import {
  compareRulePacks,
  packIdOfPackage,
  readRulePack,
  RERAIL_PUBLISHER,
  RULE_PACK_PACKAGE_NAME,
  rulePackPublisher,
  type LoadedRulePack,
  type RulePackLocalSource,
} from "./rule-packs.js";

/** The npm prefix packs are installed into: `$RERAIL_HOME/packs`. */
export function installedPacksPrefix(home: string = defaultRerailHome()): string {
  return join(home, "packs");
}

/**
 * Every pack package's directory in the prefix's own `node_modules`, by package name: `rerail-pack-<id>` and
 * `@<scope>/rerail-pack-<id>`, whoever publishes them and however npm put them there (a dependency of another package
 * is hoisted beside the ones the person installed). Only the prefix's own `node_modules` is read, so one version of a
 * package is there at a time.
 */
export function packDirectoriesOnDisk(home: string = defaultRerailHome()): string[] {
  const modules = join(installedPacksPrefix(home), "node_modules");
  if (!existsSync(modules)) return [];
  const names = readdirSync(modules, { withFileTypes: true }).flatMap(entry => {
    if (!entry.name.startsWith("@")) return [entry.name];
    return entry.isDirectory() ? readdirSync(join(modules, entry.name)).map(name => `${entry.name}/${name}`) : [];
  });
  return names.filter(name => RULE_PACK_PACKAGE_NAME.test(name)).sort(compareDeterministicStrings).map(name => join(modules, name));
}

/** The pack packages the person installed, each with the spec npm recorded: what the prefix's `package.json` lists. */
function recordedDependencies(home: string): ReadonlyMap<string, string> {
  try {
    const manifest = JSON.parse(readFileSync(join(installedPacksPrefix(home), "package.json"), "utf8")) as unknown;
    const dependencies = typeof manifest === "object" && manifest !== null && "dependencies" in manifest ? manifest.dependencies : undefined;
    if (typeof dependencies !== "object" || dependencies === null) return new Map();
    return new Map(Object.entries(dependencies).flatMap(([name, spec]) => (typeof spec === "string" ? [[name, spec] as const] : [])));
  } catch {
    return new Map();
  }
}

/**
 * Each installed pack's directory: the packs on disk (`packDirectoriesOnDisk`) the person installed, which the
 * prefix's `package.json` lists. A pack npm put there only as another package's dependency is never read, so no
 * package brings in a pack, or a package name, the person did not ask for.
 */
export function installedPackDirectories(home: string = defaultRerailHome()): string[] {
  const recorded = recordedDependencies(home);
  return packDirectoriesOnDisk(home).filter(directory => recorded.has(installedPackageName(directory)));
}

/** A tarball npm installs from (`.tgz`, `.tar.gz`, `.tar`), as against a folder. */
const TARBALL = /\.(?:tgz|tar\.gz|tar)$/iu;

/**
 * The folder or tarball each installed pack came from, by its directory, for the packs npm recorded as `file:<path>`
 * (`rerail packs add <folder | tarball>`): every other pack came from the registry.
 */
export function localSources(home: string, directories: readonly string[]): ReadonlyMap<string, RulePackLocalSource> {
  const recorded = recordedDependencies(home);
  const prefix = installedPacksPrefix(home);
  return new Map(directories.flatMap(directory => {
    const spec = recorded.get(installedPackageName(directory));
    if (spec === undefined || !spec.startsWith("file:")) return [];
    const path = resolve(prefix, spec.slice("file:".length));
    return [[directory, { kind: TARBALL.test(path) ? "tarball" : "folder", path }] as const];
  }));
}

/** The package an installed pack's directory holds: `…/node_modules/@acme/rerail-pack-sql` → `@acme/rerail-pack-sql`. */
export function installedPackageName(directory: string): string {
  const scope = basename(dirname(directory));
  return scope.startsWith("@") ? `${scope}/${basename(directory)}` : basename(directory);
}

/**
 * The package npm installed into `directory` when it is a `node_modules` package folder (`…/node_modules/<name>` or
 * `…/node_modules/@<scope>/<name>`): the name npm installed it under, which its `package.json` must repeat. Undefined
 * for any other folder, such as a Rerail checkout's `packs/<id>/`.
 */
function installedAs(directory: string): string | undefined {
  const parent = dirname(directory);
  if (basename(parent) === "node_modules") return basename(directory);
  return basename(parent).startsWith("@") && basename(dirname(parent)) === "node_modules" ? installedPackageName(directory) : undefined;
}

/**
 * The package that publishes the pack in `directory`, as far as it can be trusted: the name npm installed it under,
 * else its `package.json`'s name (`packageName`), for a folder outside `node_modules`.
 */
function trustedPackageName(directory: string, packageName: string | null): string | null {
  return installedAs(directory) ?? packageName;
}

/** The version in `package.json` at `root`, when it is core's. */
function coreVersionAt(root: string): string | undefined {
  try {
    const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as unknown;
    if (typeof manifest !== "object" || manifest === null || !("name" in manifest) || !("version" in manifest)) return undefined;
    return manifest.name === RERAIL_PACKAGE_NAME && typeof manifest.version === "string" ? manifest.version : undefined;
  } catch {
    return undefined;
  }
}

/**
 * The core version installed packs must fit: the active install's (`$RERAIL_HOME/current`), which the daemon runs,
 * else this package's. A coding agent's extension may run an older package than the active one after an update.
 */
export function activeCoreVersion(home: string = defaultRerailHome()): string {
  const version = coreVersionAt(join(home, "current")) ?? coreVersionAt(PACKAGE_ROOT);
  if (version === undefined) throw new Error(`${PACKAGE_ROOT}/package.json is not ${RERAIL_PACKAGE_NAME}'s`);
  return version;
}

export interface RuleCatalogOptions {
  /** Skip a pack whose core range (`peerDependencies["@neralabs/rerail"]`) excludes this core version. */
  readonly coreVersion?: string;
  /**
   * Skip a pack with a code rule the engine rejects. A rule refused only as `language-not-supported` loads and stays
   * inactive. Without the native engine nothing is compiled here; the checkout catalog quarantines such rules.
   */
  readonly compile?: boolean;
  /** The folder or tarball a pack directory was installed from, where it was (`localSources`). */
  readonly localSources?: ReadonlyMap<string, RulePackLocalSource>;
}

/**
 * Why a core `version` outside `range` cannot load a pack, in words for the person: which Rerail the pack is for, and
 * what to do when theirs is the older one.
 */
function coreRangeProblem(range: string | undefined, version: string): string {
  if (range === undefined) return `its package.json does not say which versions of Rerail it is for`;
  if (semver.validRange(range) !== null && semver.ltr(version, range, { includePrerelease: true })) {
    return `it needs a newer Rerail (${range}) than this one (${version}); \`rerail update\` installs the newest`;
  }
  return `it is for Rerail ${range}, and this Rerail is ${version}`;
}

/**
 * Why `loaded` cannot join a catalog whose loaded packs `holders` names by pack id (each one's package name, else its
 * directory), whose rules `owner` maps to their pack's id, and whose pack ids `reserved` keeps for Rerail's own packs
 * that are installed but left out (each one's package name), or undefined.
 */
function packProblem(
  loaded: LoadedRulePack,
  directory: string,
  claims: { readonly holders: ReadonlyMap<string, string>; readonly owner: ReadonlyMap<string, string>; readonly reserved: ReadonlyMap<string, string> },
  options: RuleCatalogOptions,
): string | undefined {
  const { pack, packageJson } = loaded;
  const { holders, owner, reserved } = claims;
  // A package.json that names another package than npm installed would borrow that package's publisher and precedence.
  const as = installedAs(directory);
  if (as !== undefined && packageJson?.name !== as) return `it is installed as ${as}, but its package.json names ${packageJson?.name ?? "no package"}`;
  const holder = holders.get(pack.id);
  if (holder !== undefined) return `pack id ${pack.id} is taken by ${holder}`;
  // Repositories add packs by id: while Rerail's own pack for an id is installed, no other pack runs in its place.
  const keeper = reserved.get(pack.id);
  if (keeper !== undefined && pack.publisher !== RERAIL_PUBLISHER) return `pack id ${pack.id} is kept for ${keeper}, which is installed but left out`;
  if (options.coreVersion !== undefined) {
    if (packageJson === undefined) return `pack ${pack.id} has no package.json naming the core versions it fits`;
    const range = packageJson.peerDependencies[RERAIL_PACKAGE_NAME];
    if (range === undefined || !semver.satisfies(options.coreVersion, range, { includePrerelease: true })) return coreRangeProblem(range, options.coreVersion);
  }
  const taken = pack.rules.find(rule => owner.has(rule.id));
  if (taken !== undefined) return `rule ${taken.id} is already in ${holders.get(owner.get(taken.id)!)}`;
  return undefined;
}

/**
 * The order packs claim their pack id and rule ids in, so which of two clashing packs loads never depends on where
 * they were found: Rerail's own first, then by package name. Packs without a package keep the order they were given.
 */
function comparePackPrecedence(left: LoadedRulePack, right: LoadedRulePack): number {
  const rank = ({ pack }: LoadedRulePack): number => (pack.publisher === RERAIL_PUBLISHER ? 0 : 1);
  return rank(left) - rank(right) || compareDeterministicStrings(left.pack.packageName ?? "", right.pack.packageName ?? "");
}

/**
 * The first code rule of `loaded` the engine rejects, or undefined; throws `EngineUnavailableError` without the engine.
 * A published pack passed the engine of the core it was released with, so on a person's machine a rule that does not
 * compile almost always means their Rerail is older than the pack: the reason says so before the engine's own words.
 */
function compileProblem(loaded: LoadedRulePack, coreVersion: string | undefined): string | undefined {
  for (const document of loaded.documents) {
    if (document.kind !== "code") continue;
    try {
      storableCodeRule(document.rule);
    } catch (error) {
      if (!(error instanceof CodeRuleCompileError)) throw error;
      const rerail = coreVersion === undefined ? "this Rerail" : `this Rerail (${coreVersion})`;
      return `${rerail} cannot run its rule ${document.rule.id}, so the pack may need a newer Rerail (\`rerail update\` installs the newest). The engine says: ${error.reason}`;
    }
  }
  return undefined;
}

/**
 * What a skipped pack's `package.json` says about it, as far as it can be read, named by the package npm installed it
 * as where there is one: a package.json naming another package is not taken at its word.
 */
function skippedPack(path: string, reason: string): SkippedRulePack {
  let packageName: string | null = null;
  let version: string | null = null;
  try {
    const manifest = JSON.parse(readFileSync(join(path, "package.json"), "utf8")) as unknown;
    if (typeof manifest === "object" && manifest !== null) {
      if ("name" in manifest && typeof manifest.name === "string") packageName = manifest.name;
      if ("version" in manifest && typeof manifest.version === "string") version = manifest.version;
    }
  } catch {
    // A pack without a readable package.json is reported by its path alone.
  }
  return { path, packageName: installedAs(path) ?? packageName, version, reason };
}

/**
 * Changes whenever a pack is added, removed, changes version or package, or any of its rule documents or `pack.json`
 * changes.
 */
export function catalogDigest(packs: readonly LoadedRulePack[]): string {
  const hash = createHash("sha256");
  for (const { pack, packJson, documents } of packs) {
    hash.update(`pack\0${pack.id}\0${String(pack.packageName)}\0${String(pack.version)}\0${stableJsonText(packJson) ?? ""}\n`);
    const digests = documents.map(document => [document.rule.id, ruleDocumentDigest(document)] as const)
      .sort(([left], [right]) => compareDeterministicStrings(left, right));
    for (const [id, digest] of digests) hash.update(`rule\0${id}\0${digest}\n`);
  }
  return `sha256:${hash.digest("hex")}`;
}

/**
 * The catalog of the packs in `packDirectories`, read from disk on every call. Each pack is read and checked on its
 * own (`readRulePack`, its package name against the one npm installed it under, then the core range and the engine
 * with `options`). Two packs may not share a pack id or a rule id: Rerail's own pack loads and the other is skipped,
 * and between two others the first by package name loads (`comparePackPrecedence`), wherever each was found. While
 * Rerail's own pack for an id is there but left out, no other pack takes that id. A skipped pack is listed with its
 * reason, in the order given, and the others load.
 */
export function readRuleCatalog(packDirectories: readonly string[], options: RuleCatalogOptions = {}): RuleCatalog {
  const read: { at: number; directory: string; loaded: LoadedRulePack }[] = [];
  const skipped: { at: number; entry: SkippedRulePack }[] = [];
  const reserved = new Map<string, string>();
  // Rerail's own pack by the package it is installed as, so its id stays its own however it fails.
  const reserve = (directory: string, packageName: string | null): void => {
    const trusted = trustedPackageName(directory, packageName);
    const id = trusted === null ? undefined : packIdOfPackage(trusted);
    if (trusted !== null && id !== undefined && rulePackPublisher(trusted) === RERAIL_PUBLISHER && !reserved.has(id)) reserved.set(id, trusted);
  };
  packDirectories.forEach((directory, at) => {
    try {
      const loaded = readRulePack(directory);
      const localSource = options.localSources?.get(directory);
      read.push({ at, directory, loaded: localSource === undefined ? loaded : { ...loaded, pack: { ...loaded.pack, localSource } } });
    } catch (error) {
      const entry = skippedPack(directory, error instanceof Error ? error.message : String(error));
      reserve(directory, entry.packageName);
      skipped.push({ at, entry });
    }
  });
  const loaded: LoadedRulePack[] = [];
  const holders = new Map<string, string>();
  const owner = new Map<string, string>();
  let compile = options.compile === true;
  for (const { at, directory, loaded: pack } of read.sort((left, right) => comparePackPrecedence(left.loaded, right.loaded))) {
    let problem = packProblem(pack, directory, { holders, owner, reserved }, options);
    if (problem === undefined && compile) {
      try {
        problem = compileProblem(pack, options.coreVersion);
      } catch (error) {
        if (!(error instanceof EngineUnavailableError)) throw error;
        compile = false;
      }
    }
    if (problem !== undefined) {
      reserve(directory, pack.pack.packageName);
      skipped.push({ at, entry: skippedPack(directory, problem) });
      continue;
    }
    holders.set(pack.pack.id, pack.pack.packageName ?? directory);
    for (const rule of pack.pack.rules) owner.set(rule.id, pack.pack.id);
    loaded.push(pack);
  }
  loaded.sort((left, right) => compareRulePacks(left.pack, right.pack));
  return {
    documents: new Map(loaded.flatMap(({ documents }) => documents.map(document => [document.rule.id, document] as const))),
    packs: new Map(loaded.map(({ pack }) => [pack.id, pack])),
    packOf: new Map(owner),
    sources: new Map(loaded.flatMap(({ sources }) => [...sources])),
    digest: catalogDigest(loaded),
    skipped: skipped.sort((left, right) => left.at - right.at).map(({ entry }) => entry),
  };
}

/**
 * A cheap stamp of what is installed and what checks it: changes when npm installs, removes or updates a pack, a pack's
 * files move, or another core version becomes active (its range decides which packs load). It does not see an edit to
 * a rule document inside a pack linked from a folder (`rerail packs add <folder>`); touch its `pack.json` to reload.
 */
function installedStamp(home: string, directories: readonly string[], coreVersion: string): string {
  const stamp = (path: string): string => {
    try {
      const info = statSync(path);
      return `${info.mtimeMs}:${info.size}`;
    } catch {
      return "-";
    }
  };
  const prefix = installedPacksPrefix(home);
  const lock = join(prefix, "node_modules", ".package-lock.json");
  return [home, coreVersion, stamp(lock), stamp(join(prefix, "package.json")), ...directories.flatMap(directory => [directory, stamp(join(directory, "package.json")), stamp(join(directory, "pack.json"))])].join("\0");
}

let installed: { readonly stamp: string; readonly catalog: RuleCatalog } | undefined;

/**
 * The catalog of the packs installed in the Rerail home `home`, checked against its active core and the engine. It is
 * read again whenever the installed packs or the active core change (`rerail packs add`, `remove`, set-up installing
 * them, `rerail update`), so a running daemon and every catalog built on it follow an install without a restart;
 * otherwise each call returns the same one.
 */
export function loadRuleCatalog(home: string = defaultRerailHome()): RuleCatalog {
  const directories = installedPackDirectories(home);
  const coreVersion = activeCoreVersion(home);
  const stamp = installedStamp(home, directories, coreVersion);
  if (installed?.stamp !== stamp) {
    installed = { stamp, catalog: readRuleCatalog(directories, { coreVersion, compile: true, localSources: localSources(home, directories) }) };
  }
  return installed.catalog;
}
