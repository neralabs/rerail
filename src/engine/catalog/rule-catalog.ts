/**
 * The rule catalog: every rule and pack Rerail can run, read from the packs installed in the Rerail home
 * (`$RERAIL_HOME/packs/node_modules/@neralabs/rerail-pack-*`). Core ships no packs. The installed files are the rules
 * (docs/rule-packs.md, "Where a pack's rules live"): nothing copies them into the database, which names packs and rules
 * by id. Each pack is checked on its own, and one that fails a check is reported in `skipped` and left out while the
 * others load.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import semver from "semver";
import { CodeRuleCompileError, EngineUnavailableError, storableCodeRule } from "../code-rule/code-rule-compile.js";
import { PACKAGE_ROOT, RERAIL_PACKAGE_NAME } from "../package-root.js";
import { compareDeterministicStrings, ruleDocumentDigest, stableJsonText } from "../stable-json.js";
import { defaultRerailHome } from "../storage/rerail-home.js";
import type { RuleCatalog, SkippedRulePack } from "./repository-model.js";
import { compareRulePacks, readRulePack, type LoadedRulePack } from "./rule-packs.js";

/** The npm prefix packs are installed into: `$RERAIL_HOME/packs`. */
export function installedPacksPrefix(home: string = defaultRerailHome()): string {
  return join(home, "packs");
}

/** Where npm puts the installed packs under the prefix. */
function installedPacksScope(home: string): string {
  return join(installedPacksPrefix(home), "node_modules", "@neralabs");
}

/**
 * Each installed pack's directory, by package name. Only the prefix's own `node_modules` is read, so one version of a
 * pack is installed at a time.
 */
export function installedPackDirectories(home: string = defaultRerailHome()): string[] {
  const scope = installedPacksScope(home);
  if (!existsSync(scope)) return [];
  return readdirSync(scope)
    .filter(name => name.startsWith("rerail-pack-"))
    .sort(compareDeterministicStrings)
    .map(name => join(scope, name));
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
  /**
   * Whether Nera shipped the pack with this id, so its rules need no approval: one installed from the npm registry
   * (`registryInstalledPacks`). Without it no pack is, and every pack rule waits for a person's approval.
   */
  readonly shipped?: (packId: string) => boolean;
}

/**
 * Ids of the packs in `home`'s prefix that npm installed from the registry, by the spec it recorded for each in the
 * prefix's `package.json`: a version, range or tag. `rerail packs add` records a pack added from a folder or tarball
 * as `file:…`, and a spec with any protocol (`git+…`, `npm:`, a URL) or a path is not the registry's either. A pack
 * with no record (copied in by hand) is not among them.
 */
export function registryInstalledPacks(home: string = defaultRerailHome()): ReadonlySet<string> {
  const scope = "@neralabs/rerail-pack-";
  let manifest: unknown;
  try {
    manifest = JSON.parse(readFileSync(join(installedPacksPrefix(home), "package.json"), "utf8"));
  } catch {
    return new Set();
  }
  const dependencies = typeof manifest === "object" && manifest !== null && "dependencies" in manifest ? manifest.dependencies : undefined;
  if (typeof dependencies !== "object" || dependencies === null) return new Set();
  return new Set(Object.entries(dependencies).flatMap(([name, spec]) => name.startsWith(scope) && typeof spec === "string"
    && spec !== "" && !spec.includes(":") && !spec.includes("/") ? [name.slice(scope.length)] : []));
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

/** Why `loaded` cannot join a catalog that already holds the packs `loadedIds` and their rules `owner`, or undefined. */
function packProblem(loaded: LoadedRulePack, loadedIds: ReadonlySet<string>, owner: ReadonlyMap<string, string>, options: RuleCatalogOptions): string | undefined {
  const { pack, packageJson } = loaded;
  if (loadedIds.has(pack.id)) return `another pack ${pack.id} is already loaded`;
  if (options.coreVersion !== undefined) {
    if (packageJson === undefined) return `pack ${pack.id} has no package.json naming the core versions it fits`;
    const range = packageJson.peerDependencies[RERAIL_PACKAGE_NAME];
    if (range === undefined || !semver.satisfies(options.coreVersion, range, { includePrerelease: true })) return coreRangeProblem(range, options.coreVersion);
  }
  const taken = pack.rules.find(rule => owner.has(rule.id));
  if (taken !== undefined) return `rule ${taken.id} is already in pack ${owner.get(taken.id)}`;
  return undefined;
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

/** What a skipped pack's `package.json` says about it, as far as it can be read. */
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
  return { path, packageName, version, reason };
}

/**
 * Changes whenever a pack is added, removed or changes version, any of its rule documents or `pack.json` changes, or
 * it becomes or stops being one Nera shipped.
 */
function catalogDigest(packs: readonly LoadedRulePack[], shipped: ReadonlySet<string>): string {
  const hash = createHash("sha256");
  for (const { pack, packJson, documents } of packs) {
    hash.update(`pack\0${pack.id}\0${String(pack.version)}\0${shipped.has(pack.id) ? "shipped" : "local"}\0${stableJsonText(packJson) ?? ""}\n`);
    const digests = documents.map(document => [document.rule.id, ruleDocumentDigest(document)] as const)
      .sort(([left], [right]) => compareDeterministicStrings(left, right));
    for (const [id, digest] of digests) hash.update(`rule\0${id}\0${digest}\n`);
  }
  return `sha256:${hash.digest("hex")}`;
}

/**
 * The catalog of the packs in `packDirectories`, read from disk on every call. Each pack is read and checked on its
 * own (`readRulePack`, then the core range and the engine with `options`); a pack whose rule another pack already
 * holds, or whose id another pack has, is skipped. A skipped pack is listed with its reason, and the others load.
 */
export function readRuleCatalog(packDirectories: readonly string[], options: RuleCatalogOptions = {}): RuleCatalog {
  const loaded: LoadedRulePack[] = [];
  const skipped: SkippedRulePack[] = [];
  const owner = new Map<string, string>();
  let compile = options.compile === true;
  for (const directory of packDirectories) {
    let pack: LoadedRulePack;
    try {
      pack = readRulePack(directory);
    } catch (error) {
      skipped.push(skippedPack(directory, error instanceof Error ? error.message : String(error)));
      continue;
    }
    let problem = packProblem(pack, new Set(loaded.map(entry => entry.pack.id)), owner, options);
    if (problem === undefined && compile) {
      try {
        problem = compileProblem(pack, options.coreVersion);
      } catch (error) {
        if (!(error instanceof EngineUnavailableError)) throw error;
        compile = false;
      }
    }
    if (problem !== undefined) {
      skipped.push(skippedPack(directory, problem));
      continue;
    }
    for (const rule of pack.pack.rules) owner.set(rule.id, pack.pack.id);
    loaded.push(pack);
  }
  loaded.sort((left, right) => compareRulePacks(left.pack, right.pack));
  const shippedPacks = new Set(loaded.filter(({ pack }) => options.shipped?.(pack.id) === true).map(({ pack }) => pack.id));
  return {
    documents: new Map(loaded.flatMap(({ documents }) => documents.map(document => [document.rule.id, document] as const))),
    packs: new Map(loaded.map(({ pack }) => [pack.id, pack])),
    packOf: new Map(owner),
    sources: new Map(loaded.flatMap(({ sources }) => [...sources])),
    shipped: new Set(loaded.filter(({ pack }) => shippedPacks.has(pack.id)).flatMap(({ documents }) => documents.map(document => document.rule.id))),
    digest: catalogDigest(loaded, shippedPacks),
    skipped,
  };
}

/**
 * A cheap stamp of what is installed and what checks it: changes when npm installs, removes or updates a pack, records
 * another spec for one, a pack's files move, or another core version becomes active (its range decides which packs
 * load). It does not see an edit to a rule document inside a pack linked from a folder (`rerail packs add <folder>`);
 * touch its `pack.json` to reload.
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
  const records = [stamp(join(prefix, "package.json")), stamp(join(prefix, "node_modules", ".package-lock.json"))];
  return [home, coreVersion, ...records, ...directories.flatMap(directory => [directory, stamp(join(directory, "package.json")), stamp(join(directory, "pack.json"))])].join("\0");
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
    const shipped = registryInstalledPacks(home);
    installed = { stamp, catalog: readRuleCatalog(directories, { coreVersion, compile: true, shipped: id => shipped.has(id) }) };
  }
  return installed.catalog;
}
