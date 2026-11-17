/**
 * Biome's configs in a repository, read as data as Biome 2.5.15 reads them (never run), for the ports that honour
 * `biome-ignore` comments: their `lint/<group>/<rule>` names.
 *
 * Biome takes the first of `biome.json`, `biome.jsonc`, `.biome.json`, `.biome.jsonc` at the root; each config file is
 * read by `biome-config-file.ts`, and a rule decided from the `linter` sections Biome merges for some files by
 * `biome-rule-decision.ts`. Globs are matched as Biome matches them, against the repository's files Biome lints with its
 * JavaScript rules: a folder holding such files decides a rule where they all agree on it and leaves it to the pack
 * where they do not; any other folder, where an override for some files or an `includes` may reach the files it gets,
 * leaves the rules they may change to the pack. A folder below the root holding a `biome.json` or `biome.jsonc`
 * (`"root": false`) is linted by that config: alone, or over the root's with `"extends": "//"`, whose overrides it then
 * matches from its own folder; there, where any config sets `includes`, a rule on keeps the pack's value. A nested root
 * config makes Biome refuse to run, and Rerail reads nothing. What a config takes and Rerail cannot read (a package in
 * `extends`, a file outside the repository, a root config above the repository) leaves the rules it may set at the
 * pack's values.
 */
import { posix } from "node:path";
import { codeRuleDialectFamily, codeRuleDialectForPath } from "../../../code-rule/code-rule-languages.js";
import { globPattern } from "../../rule-pack-matcher.js";
import { repositoryFiles } from "../repository-files.js";
import {
  readRootFile,
  type FileRead,
  type FolderDecision,
  inFolder,
  settlesAlike,
  type LinterConfig,
  type LinterNote,
  type LinterReader,
  type Port,
  type PortDecision,
  type PortVerdict,
} from "../linter-config.js";
import { configOf, mergedFilters, type ConfigFile, type Filter, type Filters, type LinterLayer, type Override } from "./biome-config-file.js";
import { holdsFile, matchesGlobs } from "./biome-globs.js";
import { nameDecision, portDecision, sharedDecision, UNREAD, type Context, type Step, type Unread } from "./biome-rule-decision.js";

/** Biome's config file names in the order it looks for them in a folder (`ConfigName` in crates/biome_fs/src/fs.rs l.24-25). */
const BIOME_FILES = ["biome.json", "biome.jsonc", ".biome.json", ".biome.jsonc"];
/** The config file names Biome 2.5.15 takes in a folder below the root, in the order it looks for them. */
const NESTED_FILES = ["biome.json", "biome.jsonc"];
/** The families of the files Biome lints with its JavaScript rules: JavaScript, TypeScript and their JSX, and Vue's script blocks. */
const BIOME_FAMILIES: readonly string[] = ["ecma", "vue"];

/**
 * A file in `extends` of a config in folder `base`, relative to the root; undefined for a package or a file outside the
 * repository. A relative entry (`is_relative_specifier`, crates/biome_resolver/src/lib.rs l.865-867) is a path from
 * the config's folder; any other is, in the root config, first tried as a path from the working directory
 * (`with_assume_relative`), taken here as the root, then as a package, and in a nested config a package (Biome 2.5.15
 * resolves no other there).
 */
function extendedPath(root: string, base: string, entry: string): string | undefined {
  const relative = /^\.\.?(?:\/|$)/u.test(entry);
  if (!relative && base !== "") return undefined;
  const path = posix.normalize(posix.join(base, entry));
  if (/^(?:\/|\.\.(?:\/|$))/u.test(path)) return undefined;
  return relative || readRootFile(root, path).present ? path : undefined;
}

/** The `linter` of a config file that has none: Biome's defaults. */
const NO_LINTER: LinterLayer = { domains: new Map() };

/** What one config file's `extends` merges under it. */
interface Extended {
  /** Each config's `linter` (not its own `extends`, which Biome does not follow), or `UNREAD`, in order. */
  readonly layers: readonly (LinterLayer | Unread)[];
  readonly overrides: readonly Override[];
  readonly filters: readonly Filters[];
  readonly unread: string | undefined;
}

/** What `extends` of `file`, in folder `base`, merges under it, in order. */
function extendedLayers(root: string, base: string, file: string, config: ConfigFile, notes: LinterNote[]): Extended {
  const layers: (LinterLayer | Unread)[] = [];
  const overrides: Override[] = [];
  const filters: Filters[] = [];
  let unread: string | undefined;
  for (const entry of config.extends) {
    const extended = extendedConfig(root, base, file, entry, notes);
    if (extended === undefined) {
      unread = `${file} extends ${JSON.stringify(entry)}, which Rerail does not read`;
      layers.push(UNREAD);
      filters.push({});
      continue;
    }
    layers.push(extended.linter ?? NO_LINTER);
    overrides.push(...extended.overrides);
    filters.push(extended.filters);
  }
  return { layers, overrides, filters, unread };
}

/** Config file `entry` of `extends` of `file`, in folder `base`, if Rerail reads it; a note saying why not otherwise. */
function extendedConfig(root: string, base: string, file: string, entry: string, notes: LinterNote[]): ConfigFile | undefined {
  const path = extendedPath(root, base, entry);
  if (path === undefined) notes.push({ linter: "biome", text: `${file}: extends ${JSON.stringify(entry)} is not read: it is a package or a file outside the repository; the rules it may set keep the pack's values.` });
  return path === undefined ? undefined : configOf(path, readRootFile(root, path), notes);
}

/** Whether Biome lints some files: surely; not, by `because`; or perhaps, Rerail cannot tell (`because`). */
type Linted = { readonly state: "yes" } | { readonly state: "no" | "maybe"; readonly because: string };

const LINTED: Linted = { state: "yes" };

/** `decision` for files Biome lints as `linted`: off where it lints none, unknown for on where it may not lint them. */
function lintedDecision(decision: PortDecision, linted: Linted): PortDecision {
  if (linted.state === "yes") return decision;
  if (linted.state === "no") return { on: false, explicit: false, because: linted.because };
  return decision.on === true ? { on: "unknown", because: linted.because } : decision;
}

/**
 * The config Biome lints a folder by (`""`: the root's, for every file outside the others): what it merges, in order,
 * its overrides, by folders relative to the root, and the `includes` the files it lints must match.
 */
interface ReadScope {
  readonly folder: string;
  readonly context: Context;
  readonly layers: readonly (LinterLayer | Unread)[];
  readonly overrides: readonly Override[];
  readonly filters: readonly Filter[];
}

/** A folder's config, or why Rerail cannot tell it. */
type BiomeScope = ReadScope | { readonly folder: string; readonly unknown: string };

/**
 * Whether Biome lints file `path` (relative to the root) of `scope`, or, without one, any file of it. Rerail matches
 * `includes` only in the root config's scope: below it, Biome takes the root's `linter.includes` and not its `files.includes`.
 */
function lintedIn(scope: ReadScope, path?: string): Linted {
  if (scope.filters.length === 0) return LINTED;
  const labels = scope.filters.map(filter => filter.label).join(" and ");
  if (scope.folder !== "" || path === undefined) return { state: "maybe", because: `${labels} may leave files here unlinted` };
  const leaving = scope.filters.find(filter => !holdsFile(filter.globs, path));
  return leaving === undefined ? LINTED : { state: "no", because: `${leaving.label} leaves the files here unlinted` };
}

/** What Biome merges for some files, and whether it lints them; or why Rerail cannot tell. */
type Reading = { readonly context: Context; readonly steps: readonly Step[]; readonly linted: Linted } | { readonly unknown: string };

function readingDecision(reading: Reading, names: readonly string[]): PortDecision {
  if ("unknown" in reading) return { on: "unknown", because: reading.unknown };
  return lintedDecision(portDecision(names.map(name => nameDecision(reading.steps, name, reading.context))), reading.linted);
}

const within = (path: string, folder: string): boolean => folder === "" || path === folder || path.startsWith(`${folder}/`);

/** `override`, from a config in `folder`, with its folders relative to the root. */
const rebased = (override: Override, folder: string): Override =>
  ({ ...override, folders: override.folders?.map(path => [folder, path].filter(part => part !== "").join("/")) });

/**
 * What Biome merges for a file in `folder` Rerail knows nothing more of: the deepest config holding it, its overrides
 * for a folder holding `folder`, and those for some files, which may apply.
 */
function folderReading(scopes: readonly BiomeScope[], folder: string): Reading {
  const scope = scopes.findLast(candidate => within(folder, candidate.folder))!;
  if ("unknown" in scope) return scope;
  const overrides = scope.overrides.filter(override => override.folders?.some(holder => within(folder, holder)) ?? true);
  const steps = [...scope.layers.map((layer): Step => ({ layer })), ...overrides.map(override => ({ layer: override.layer, ...override.folders === undefined ? { some: override.where } : {} }))];
  return { context: scope.context, steps, linted: lintedIn(scope) };
}

/** A file Biome lints with its JavaScript rules; files alike (`key`: the same config, overrides and linting) decide alike. */
interface BiomeFile {
  readonly path: string;
  readonly folder: string;
  readonly key: string;
  readonly reading: Reading;
}

/** File `path` of the repository, as the deepest of `scopes` holding it reads for it: its config, then the overrides whose `includes` hold it. */
function biomeFile(scopes: readonly BiomeScope[], path: string): BiomeFile {
  const folder = posix.dirname(path) === "." ? "" : posix.dirname(path);
  const index = scopes.findLastIndex(candidate => within(folder, candidate.folder));
  const scope = scopes[index]!;
  if ("unknown" in scope) return { path, folder, key: `${index}`, reading: scope };
  const relative = scope.folder === "" ? path : path.slice(scope.folder.length + 1);
  const overrides = scope.overrides.filter(override => matchesGlobs(override.includes, relative, false));
  const linted = lintedIn(scope, path);
  return {
    path,
    folder,
    key: JSON.stringify([index, overrides.map(override => scope.overrides.indexOf(override)), linted]),
    reading: { context: scope.context, steps: [...scope.layers, ...overrides.map(override => override.layer)].map(layer => ({ layer })), linted },
  };
}

/** The files of `files` a port's `scope` holds, by folder, one of each kind (`key`) per folder. */
function filesByFolder(files: readonly BiomeFile[], scope: Port["scope"]): ReadonlyMap<string, readonly BiomeFile[]> {
  const include = globPattern(scope?.include ?? ["**/*"]);
  const exclude = globPattern(scope?.exclude ?? []);
  const byFolder = new Map<string, BiomeFile[]>();
  for (const file of files.filter(candidate => include.test(candidate.path) && !exclude.test(candidate.path))) {
    const found = byFolder.get(file.folder) ?? [];
    if (!found.some(other => other.key === file.key)) found.push(file);
    byFolder.set(file.folder, found);
  }
  return byFolder;
}

/** The decision files of one folder come to: the state they share, else the pack's. */
function filesDecision(decisions: readonly PortDecision[]): PortDecision {
  const differing = decisions.find(decision => decision.on !== decisions[0]!.on);
  return sharedDecision(decisions) ?? { on: "unknown", because: `${decisions[0]!.because}; ${differing!.because}: each for only some of the files here` };
}

/**
 * The port's decision for every file, and per folder where it differs from the nearest folder holding it (or, in none,
 * from every file's): a folder holding files the port runs on decides as they do, any other as the deepest config
 * holding it. `structural` are the folders a config or an override for whole folders is for.
 */
function portVerdict(scopes: readonly BiomeScope[], byFolder: ReadonlyMap<string, readonly BiomeFile[]>, structural: readonly string[], names: readonly string[]): PortVerdict {
  const byKey = new Map<string, PortDecision>();
  const fileDecision = (file: BiomeFile): PortDecision => {
    const decided = byKey.get(file.key) ?? readingDecision(file.reading, names);
    byKey.set(file.key, decided);
    return decided;
  };
  const decisionIn = (folder: string): PortDecision => {
    const files = byFolder.get(folder);
    return files === undefined ? readingDecision(folderReading(scopes, folder), names) : filesDecision(files.map(fileDecision));
  };
  const decision = decisionIn("");
  const folders: FolderDecision[] = [];
  // Sorted, a folder comes after every folder holding it.
  for (const folder of [...new Set([...structural, ...byFolder.keys()])].filter(path => path !== "").sort()) {
    const here = decisionIn(folder);
    const holder = folders.findLast(known => within(folder, known.folder)) ?? decision;
    if (!settlesAlike(here, holder)) folders.push(inFolder(folder, here));
  }
  return { decision, ...folders.length > 0 ? { folders } : {} };
}

/** The verdicts of Biome's configs, the root's first and nested ones after the folders holding them, over the repository's `files`. */
function biomeVerdicts(scopes: readonly BiomeScope[], files: readonly BiomeFile[]): LinterConfig {
  const structural = scopes.flatMap(scope => [scope.folder, ...("unknown" in scope ? [] : scope.overrides.flatMap(override => override.folders ?? []))]);
  const byScope = new Map<string, ReadonlyMap<string, readonly BiomeFile[]>>();
  return {
    verdict(port: Port): PortVerdict | undefined {
      const names = port.suppress.biome;
      if (names.length === 0) return undefined;
      const key = JSON.stringify(port.scope ?? null);
      const byFolder = byScope.get(key) ?? filesByFolder(files, port.scope);
      byScope.set(key, byFolder);
      return portVerdict(scopes, byFolder, structural, names);
    },
  };
}

/** The scope of config `config`, file `file` in `folder`, and what it extends, read as Biome merges them. */
function scopeOf(root: string, folder: string, file: string, config: ConfigFile, manifests: boolean, notes: LinterNote[]): ReadScope {
  const extended = extendedLayers(root, folder, file, config, notes);
  return {
    folder,
    context: { file, unread: extended.unread ?? `${file} takes a config Rerail does not read`, manifests },
    layers: [...extended.layers, config.linter ?? NO_LINTER],
    overrides: [...extended.overrides, ...config.overrides].map(override => rebased(override, folder)),
    filters: mergedFilters([...extended.filters, config.filters]),
  };
}

/**
 * The nested config `file` (`root: false`): by itself, or with `extends: "//"` over the root's, whose overrides it
 * matches from its own folder (Biome 2.5.15); `"refused"` (with a note) for a root config, with which Biome runs nothing.
 * The root config's `includes` may narrow the files it lints too.
 */
function nestedScope(root: string, file: string, rootScope: ReadScope, notes: LinterNote[]): BiomeScope | "refused" {
  const folder = posix.dirname(file);
  const config = configOf(file, readRootFile(root, file), notes);
  if (config === undefined) return { folder, unknown: `${file} is not read` };
  if (config.root) {
    notes.push({ linter: "biome", text: `${file} is not read: it is a root config below the root's, which Biome 2.5.15 refuses, so it runs no rule.` });
    return "refused";
  }
  if (!config.extendsRoot) {
    const scope = scopeOf(root, folder, file, config, rootScope.context.manifests, notes);
    return { ...scope, filters: [...rootScope.filters, ...scope.filters] };
  }
  return {
    folder,
    context: { ...rootScope.context, file },
    layers: [...rootScope.layers, config.linter ?? NO_LINTER],
    overrides: [...rootScope.overrides, ...config.overrides].map(override => rebased(override, folder)),
    filters: [...rootScope.filters, ...mergedFilters([config.filters])],
  };
}

/** Biome's nested configs: in each folder below the root, its `biome.json` or else its `biome.jsonc` (Biome 2.5.15 takes no `.biome.json` there). */
function nestedFiles(files: readonly string[]): string[] {
  const nested = files.filter(path => path.includes("/") && NESTED_FILES.includes(posix.basename(path)));
  return [...Map.groupBy(nested, path => posix.dirname(path)).values()]
    .map(found => found.sort((left, right) => NESTED_FILES.indexOf(posix.basename(left)) - NESTED_FILES.indexOf(posix.basename(right)))[0]!)
    .sort();
}

/** Biome's config `file` at `root`, as `read` holds it, the nested configs below it, and what each extends, read as Biome merges them. */
function readBiomeConfig(root: string, file: string, read: FileRead, notes: LinterNote[]): LinterConfig | undefined {
  const config = configOf(file, read, notes);
  if (config === undefined) return undefined;
  if (!config.root) {
    notes.push({ linter: "biome", text: `${file} is not read: it is not a root config ("root": false, or "extends": "//"), so Biome takes a config outside the repository.` });
    const because = `${file} is not Biome's root config`;
    return { verdict: port => (port.suppress.biome.length === 0 ? undefined : { decision: { on: "unknown", because } }) };
  }
  const files = repositoryFiles(root);
  const rootScope = scopeOf(root, "", file, config, files.some(path => posix.basename(path) === "package.json"), notes);
  const nested = nestedFiles(files).map(path => nestedScope(root, path, rootScope, notes));
  if (nested.includes("refused")) return undefined;
  const scopes = [rootScope, ...nested as BiomeScope[]];
  const linted = files.filter(path => {
    const dialect = codeRuleDialectForPath(path);
    return dialect !== undefined && BIOME_FAMILIES.includes(codeRuleDialectFamily(dialect));
  });
  return biomeVerdicts(scopes, linted.map(path => biomeFile(scopes, path)));
}

/** Biome's config, read as Biome 2.5.15 reads it at the repository root: the first of its config files there. */
export const biomeReader: LinterReader = {
  linter: "biome",
  kinds: ["biome"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    for (const file of BIOME_FILES) {
      const read = readRootFile(root, file);
      if (read.present) return readBiomeConfig(root, file, read, notes);
    }
    return undefined;
  },
};
