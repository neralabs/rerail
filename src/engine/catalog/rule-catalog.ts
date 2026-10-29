/**
 * The rule catalog: every rule and pack Rerail can run, read from the packs installed in the Rerail home
 * (`$RERAIL_HOME/packs/node_modules`: `rerail-pack-<id>` and `@<scope>/rerail-pack-<id>`, Rerail's own under
 * `@neralabs`). Core ships no packs. The installed files are the rules (docs/rule-packs.md, "Where a pack's rules
 * live"): nothing copies them into the database, which names packs and rules by id. Each pack is checked on its own,
 * and one that fails a check is reported in `skipped` and left out while the others load.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import semver from "semver";
import { CodeRuleCompileError, EngineUnavailableError, storableCodeRule } from "../code-rule/code-rule-compile.js";
import { PACKAGE_ROOT, RERAIL_PACKAGE_NAME } from "../package-root.js";
import { compareDeterministicStrings, ruleDocumentDigest, stableJsonText } from "../stable-json.js";
import { defaultRerailHome } from "../storage/rerail-home.js";
import type { RuleCatalog, SkippedRulePack } from "./repository-model.js";
import { compareRulePacks, readRulePack, RERAIL_PUBLISHER, RULE_PACK_PACKAGE_NAME, type LoadedRulePack } from "./rule-packs.js";

/** The npm prefix packs are installed into: `$RERAIL_HOME/packs`. */
export function installedPacksPrefix(home: string = defaultRerailHome()): string {
  return join(home, "packs");
}

/**
 * Each installed pack's directory, by package name: `node_modules/rerail-pack-<id>` and
 * `node_modules/@<scope>/rerail-pack-<id>` under the prefix, whoever publishes them. Only the prefix's own
 * `node_modules` is read, so one version of a package is installed at a time.
 */
export function installedPackDirectories(home: string = defaultRerailHome()): string[] {
  const modules = join(installedPacksPrefix(home), "node_modules");
  if (!existsSync(modules)) return [];
  const names = readdirSync(modules, { withFileTypes: true }).flatMap(entry => {
    if (!entry.name.startsWith("@")) return [entry.name];
    return entry.isDirectory() ? readdirSync(join(modules, entry.name)).map(name => `${entry.name}/${name}`) : [];
  });
  return names.filter(name => RULE_PACK_PACKAGE_NAME.test(name)).sort(compareDeterministicStrings).map(name => join(modules, name));
}

/** The package an installed pack's directory holds: `…/node_modules/@acme/rerail-pack-sql` → `@acme/rerail-pack-sql`. */
export function installedPackageName(directory: string): string {
  const scope = basename(dirname(directory));
  return scope.startsWith("@") ? `${scope}/${basename(directory)}` : basename(directory);
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
 * directory) and whose rules `owner` maps to their pack's id, or undefined.
 */
function packProblem(loaded: LoadedRulePack, holders: ReadonlyMap<string, string>, owner: ReadonlyMap<string, string>, options: RuleCatalogOptions): string | undefined {
  const { pack, packageJson } = loaded;
  const holder = holders.get(pack.id);
  if (holder !== undefined) return `pack id ${pack.id} is taken by ${holder}`;
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
 * Changes whenever a pack is added, removed, changes version or package, or any of its rule documents or `pack.json`
 * changes.
 */
function catalogDigest(packs: readonly LoadedRulePack[]): string {
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
 * own (`readRulePack`, then the core range and the engine with `options`). Two packs may not share a pack id or a
 * rule id: Rerail's own pack loads and the other is skipped, and between two others the first by package name loads
 * (`comparePackPrecedence`), wherever each was found. A skipped pack is listed with its reason, in the order given,
 * and the others load.
 */
export function readRuleCatalog(packDirectories: readonly string[], options: RuleCatalogOptions = {}): RuleCatalog {
  const read: { at: number; directory: string; loaded: LoadedRulePack }[] = [];
  const skipped: { at: number; entry: SkippedRulePack }[] = [];
  packDirectories.forEach((directory, at) => {
    try {
      read.push({ at, directory, loaded: readRulePack(directory) });
    } catch (error) {
      skipped.push({ at, entry: skippedPack(directory, error instanceof Error ? error.message : String(error)) });
    }
  });
  const loaded: LoadedRulePack[] = [];
  const holders = new Map<string, string>();
  const owner = new Map<string, string>();
  let compile = options.compile === true;
  for (const { at, directory, loaded: pack } of read.sort((left, right) => comparePackPrecedence(left.loaded, right.loaded))) {
    let problem = packProblem(pack, holders, owner, options);
    if (problem === undefined && compile) {
      try {
        problem = compileProblem(pack, options.coreVersion);
      } catch (error) {
        if (!(error instanceof EngineUnavailableError)) throw error;
        compile = false;
      }
    }
    if (problem !== undefined) {
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
  const lock = join(installedPacksPrefix(home), "node_modules", ".package-lock.json");
  return [home, coreVersion, stamp(lock), ...directories.flatMap(directory => [directory, stamp(join(directory, "package.json")), stamp(join(directory, "pack.json"))])].join("\0");
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
    installed = { stamp, catalog: readRuleCatalog(directories, { coreVersion, compile: true }) };
  }
  return installed.catalog;
}
