/**
 * Biome's configs in a repository, read as data as Biome 2.5.15 reads them (never run), for the ports that honour
 * `biome-ignore` comments: their `lint/<group>/<rule>` names.
 *
 * Biome takes the first of `biome.json`, `biome.jsonc`, `.biome.json`, `.biome.jsonc` at the root. Its `linter`
 * section decides which rules run: `enabled`, `rules` (a preset, `recommended`, groups set to a severity or listing
 * rules) and `domains`; `extends` merges other config files under it and `overrides` merges more `linter` sections
 * over it for the files their `includes` hold; `files.includes` and `linter.includes` leave the files they do not hold
 * unlinted. Globs are matched as Biome matches them, against the repository's files Biome lints with its JavaScript
 * rules: a folder holding such files decides a rule where they all agree on it and leaves it to the pack where they do
 * not; any other folder, where an override for some files or an `includes` may reach the files it gets, leaves the
 * rules they may change to the pack. Which rules a preset or a domain holds comes from Biome's own rule metadata
 * (`biome-rules.generated.ts`); a domain's recommended rules also turn on for the dependencies of a `package.json`,
 * which Rerail does not read, so they keep the pack's values in a repository with one. A folder below the root holding
 * a `biome.json` or `biome.jsonc` (`"root": false`) is linted by that config: alone, or over the root's with
 * `"extends": "//"`, whose overrides it then matches from its own folder; there, where any config sets `includes`,
 * a rule on keeps the pack's value. A nested root config makes Biome refuse to run, and Rerail reads nothing. What a
 * config takes and Rerail cannot read (a package in `extends`, a file outside the repository, a root config above the
 * repository) leaves the rules it may set at the pack's values.
 */
import { posix } from "node:path";
import { codeRuleDialectFamily, codeRuleDialectForPath } from "../../../code-rule/code-rule-languages.js";
import { globPattern } from "../../rule-pack-matcher.js";
import { BIOME_DOMAINS, BIOME_RULES, type BiomeRuleDeclaration } from "../generated/biome-rules.generated.js";
import { folderOfGlob } from "./eslint-flat-config.js";
import { repositoryFiles } from "../repository-files.js";
import { LintConfigFormatError, readFormat } from "../formats/lint-config-formats.js";
import { parseJsonc } from "../formats/jsonc-format.js";
import {
  NOT_A_FILE,
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

/** Biome's config file names in the order it looks for them in a folder (`ConfigName` in crates/biome_fs/src/fs.rs l.24-25). */
const BIOME_FILES = ["biome.json", "biome.jsonc", ".biome.json", ".biome.jsonc"];
/** The config file names Biome 2.5.15 takes in a folder below the root, in the order it looks for them. */
const NESTED_FILES = ["biome.json", "biome.jsonc"];
/** The families of the files Biome lints with its JavaScript rules: JavaScript, TypeScript and their JSX, and Vue's script blocks. */
const BIOME_FAMILIES: readonly string[] = ["ecma", "vue"];
/** A rule's or a group's severity (`RulePlainConfiguration`, crates/biome_configuration/src/analyzer/mod.rs l.287-298). */
const LEVELS = ["off", "on", "info", "warn", "error"];
/** A `preset` (`PresetConfig`, crates/biome_configuration/src/analyzer/presets.rs). */
const PRESETS = ["recommended", "all", "none"];
/** A domain's value (`RuleDomainValue`, crates/biome_configuration/src/analyzer/linter/mod.rs l.71-81). */
const DOMAIN_VALUES = ["all", "none", "recommended"];
/** A rule object's keys (`RuleWithOptions`, `RuleWithFixOptions`, mod.rs l.474-531). */
const RULE_OBJECT_KEYS = ["level", "options", "fix"];
/** A group's keys besides its rules, and the top of `rules`' besides its groups. */
const PRESET_KEYS = ["recommended", "preset"];
/** Each group's rules. */
const GROUP_RULES = new Map<string, Set<string>>();
for (const key of Object.keys(BIOME_RULES)) {
  const [group = "", rule = ""] = key.split("/");
  GROUP_RULES.set(group, (GROUP_RULES.get(group) ?? new Set()).add(rule));
}
/** Biome turns nursery rules on only by name or by the group: no preset, domain or dependency does. */
const NURSERY = "nursery";
const OPTIONS_NOT_MAPPED = "Biome's rule options are not read";

type Level = "off" | "on" | "info" | "warn" | "error";
type Preset = "recommended" | "all" | "none";
type DomainValue = "all" | "none" | "recommended";

/** A value the config sets, with where it sets it, for the person. */
interface Fact<T> {
  readonly value: T;
  readonly by: string;
}

/** What an extended config Rerail does not read may have set: any value, or none. */
const UNREAD = Symbol("unread");
type Unread = typeof UNREAD;
/** A value as far as Rerail knows it: set, unset, or possibly set by a config it does not read. */
type Known<T> = Fact<T> | Unread | undefined;

const isFact = <T>(known: Known<T>): known is Fact<T> => known !== undefined && known !== UNREAD;

/** A rule's entry in a group: its severity, and whether it sets options. */
interface RuleEntry {
  readonly level: Level;
  readonly options: boolean;
}

/** A group in one `rules` object: a severity for all its rules, or its own preset and rule entries. */
type GroupLayer =
  | { readonly kind: "plain"; readonly level: Fact<Level> }
  | { readonly kind: "object"; readonly recommended?: Fact<boolean>; readonly preset?: Fact<Preset>; readonly rules: ReadonlyMap<string, Fact<RuleEntry>> };

/** One `rules` object. */
interface RulesLayer {
  readonly recommended?: Fact<boolean>;
  readonly preset?: Fact<Preset>;
  readonly groups: ReadonlyMap<string, GroupLayer>;
}

/** One `linter` section Biome merges: a config file's, or an override's. */
interface LinterLayer {
  readonly enabled?: Fact<boolean>;
  readonly rules?: RulesLayer;
  readonly domains: ReadonlyMap<string, Fact<DomainValue>>;
}

/** An override's `linter` section, for the files its `includes` hold, from the folder of the config it is in. */
interface Override {
  /** `<file>: overrides[<index>]`, for the person. */
  readonly where: string;
  readonly includes: Globs;
  /** The whole folders `includes` names (`""`: every file), relative to the root; undefined where it holds other files. */
  readonly folders: readonly string[] | undefined;
  readonly layer: LinterLayer;
}

/** A list of Biome globs, each a pattern and whether it is negated (`!` or `!!`). */
type Globs = readonly { readonly negated: boolean; readonly pattern: RegExp }[];

/** A config's `files.includes` or `linter.includes`: Biome lints only the files it holds. */
interface Filter {
  readonly label: string;
  readonly globs: Globs;
}

/** The keys whose `includes` narrow the files Biome lints, each a list the configs `extends` merges concatenate. */
const FILTER_KEYS = ["files", "linter"] as const;
type Filters = Partial<Record<(typeof FILTER_KEYS)[number], Filter>>;

/** One config file as read: whether it is a root config, what it extends, its `linter` and its overrides. */
interface ConfigFile {
  readonly root: boolean;
  /** `extends: "//"`: a nested config that merges over the root config. */
  readonly extendsRoot: boolean;
  readonly extends: readonly string[];
  readonly linter: LinterLayer | undefined;
  readonly overrides: readonly Override[];
  readonly filters: Filters;
}

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === "object" && value !== null && !Array.isArray(value);

function fail(where: string, what: string): never {
  throw new LintConfigFormatError(`${where} ${what}`);
}

function objectAt(value: unknown, where: string): Record<string, unknown> {
  if (!isObject(value)) fail(where, "is not an object");
  return value;
}

function oneOf<T extends string>(value: unknown, allowed: readonly string[], where: string): T {
  if (typeof value !== "string" || !allowed.includes(value)) fail(where, `is not one of ${allowed.map(text => JSON.stringify(text)).join(", ")}`);
  return value as T;
}

function flag(value: unknown, where: string): boolean {
  if (typeof value !== "boolean") fail(where, "is not true or false");
  return value;
}

/** `value`, set at `key` in `path` of `file`, as the person reads it: `biome.json: "noDebugger": "off" in linter.rules.suspicious`. */
const factOf = <T>(value: T, file: string, path: string, key: string, shown: unknown = value): Fact<T> =>
  ({ value, by: `${file}: ${JSON.stringify(key)}: ${JSON.stringify(shown)} in ${path}` });

/** A group's or `rules`' own `recommended` and `preset`. */
function presetSettings(object: Record<string, unknown>, file: string, path: string): { recommended?: Fact<boolean>; preset?: Fact<Preset> } {
  return {
    ...(object.recommended === undefined ? {} : { recommended: factOf(flag(object.recommended, `${path}.recommended`), file, path, "recommended") }),
    ...(object.preset === undefined ? {} : { preset: factOf(oneOf<Preset>(object.preset, PRESETS, `${path}.preset`), file, path, "preset") }),
  };
}

/** A rule's entry: a severity, or `{ level, options, fix }`. */
function ruleEntry(value: unknown, where: string): RuleEntry {
  if (typeof value === "string") return { level: oneOf<Level>(value, LEVELS, where), options: false };
  const entry = objectAt(value, where);
  const unknown = Object.keys(entry).find(key => !RULE_OBJECT_KEYS.includes(key));
  if (unknown !== undefined) fail(where, `has an unknown key ${JSON.stringify(unknown)}`);
  const options = isObject(entry.options) ? Object.keys(entry.options).length > 0 : entry.options !== undefined;
  return { level: oneOf<Level>(entry.level, LEVELS, `${where}.level`), options };
}

/** Group `group` in the `rules` at `path`. */
function groupLayer(value: unknown, file: string, path: string, group: string): GroupLayer {
  if (typeof value === "string") return { kind: "plain", level: factOf(oneOf<Level>(value, LEVELS, `${path}.${group}`), file, path, group) };
  const groupPath = `${path}.${group}`;
  const object = objectAt(value, groupPath);
  const rules = new Map<string, Fact<RuleEntry>>();
  for (const [key, entry] of Object.entries(object).filter(([key]) => !PRESET_KEYS.includes(key))) {
    if (GROUP_RULES.get(group)?.has(key) !== true) fail(groupPath, `has no rule ${JSON.stringify(key)} in Biome 2.5.15`);
    const parsed = ruleEntry(entry, `${groupPath}.${key}`);
    rules.set(key, factOf(parsed, file, groupPath, key, parsed.level));
  }
  return { kind: "object", ...presetSettings(object, file, groupPath), rules };
}

function rulesLayer(value: unknown, file: string, path: string): RulesLayer {
  const object = objectAt(value, path);
  const groups = new Map<string, GroupLayer>();
  for (const [key, group] of Object.entries(object).filter(([key]) => !PRESET_KEYS.includes(key))) {
    if (!GROUP_RULES.has(key)) fail(path, `has no group ${JSON.stringify(key)} in Biome 2.5.15`);
    groups.set(key, groupLayer(group, file, path, key));
  }
  return { ...presetSettings(object, file, path), groups };
}

function domainsLayer(value: unknown, file: string, path: string): ReadonlyMap<string, Fact<DomainValue>> {
  const object = objectAt(value ?? {}, path);
  return new Map(Object.entries(object).map(([domain, setting]) => {
    if (!BIOME_DOMAINS.includes(domain)) fail(path, `has no domain ${JSON.stringify(domain)} in Biome 2.5.15`);
    return [domain, factOf(oneOf<DomainValue>(setting, DOMAIN_VALUES, `${path}.${domain}`), file, path, domain)];
  }));
}

/** The `linter` section at `path` of `file`. */
function linterLayer(value: unknown, file: string, path: string): LinterLayer {
  const linter = objectAt(value, path);
  return {
    ...(linter.enabled === undefined ? {} : { enabled: factOf(flag(linter.enabled, `${path}.enabled`), file, path, "enabled") }),
    ...(linter.rules === undefined ? {} : { rules: rulesLayer(linter.rules, file, `${path}.rules`) }),
    domains: domainsLayer(linter.domains, file, `${path}.domains`),
  };
}

/** The globs Biome matches every file with: an override for them is for the whole folder of its config. */
const EVERY_FILE = new Set(["**", "**/*"]);

/** A glob's piece at one index: its regular expression's source, how many characters after it it takes, and how it nests alternates. */
interface GlobToken {
  readonly source: string;
  readonly skip: number;
  readonly depth: number;
}

const literal = (text: string, skip = 0): GlobToken => ({ source: text.replace(/[$()*+./?[\\\]^{|}]/gu, String.raw`\$&`), skip, depth: 0 });

/** `{`, `,` and `}` by the alternates' depth they are at: Biome refuses nested alternates and a `}` closing none. */
const ALTERNATES: Readonly<Record<string, GlobToken>> = {
  "{0": { source: "(?:", skip: 0, depth: 1 },
  ",0": literal(","),
  ",1": { source: "|", skip: 0, depth: 0 },
  "}1": { source: ")", skip: 0, depth: -1 },
};
/** The characters `\` escapes in a Biome glob; any other after it, or none, Biome refuses. */
const ESCAPED = [..."!*?{}[]\\"];

/** Whether `**` at `index` of `glob` starts a segment, not right after another `**` (`validate_glob`'s `allow_globstar`). */
const segmentStart = (glob: string, index: number): boolean => index === 0 || (glob[index - 1] === "/" && !glob.slice(0, index).endsWith("**/"));

/** `*` at `index` of `glob`, within a segment; or `**`, any segments, where it is a whole segment. */
function starToken(glob: string, index: number): GlobToken | undefined {
  if (glob[index + 1] !== "*") return { source: "[^/]*", skip: 0, depth: 0 };
  const after = glob[index + 2];
  if (!segmentStart(glob, index) || (after !== undefined && after !== "/")) return undefined;
  return after === undefined ? { source: ".*", skip: 1, depth: 0 } : { source: "(?:[^/]*/)*", skip: 2, depth: 0 };
}

/** The token at `index` of `glob`, `depth` alternates in; undefined where Biome refuses it. */
function globToken(glob: string, index: number, depth: number): GlobToken | undefined {
  const character = glob[index]!;
  if (character === "\\") return ESCAPED.includes(glob.charAt(index + 1)) ? literal(glob.charAt(index + 1), 1) : undefined;
  if (character === "*") return starToken(glob, index);
  if ("?[]".includes(character)) return undefined;
  return "{,}".includes(character) ? ALTERNATES[`${character}${depth}`] : literal(character);
}

/**
 * Glob `glob` (without its `!`) as a regular expression's source, as Biome 2.5.15 reads it (`validate_glob`,
 * crates/biome_glob/src/lib.rs l.638-704, over globset with `literal_separator`); undefined where Biome refuses it.
 */
function globSource(glob: string): string | undefined {
  let source = "";
  let depth = 0;
  for (let index = 0; index < glob.length; index += 1) {
    const token = globToken(glob, index, depth);
    if (token === undefined) return undefined;
    source += token.source;
    depth += token.depth;
    index += token.skip;
  }
  return depth === 0 ? source : undefined;
}

/** The globs at `where`, each with a leading `./` dropped (`NormalizedGlob`) and then a `!` or `!!` read as negating it. */
function globsAt(value: unknown, where: string): Globs {
  if (!Array.isArray(value) || !value.every(glob => typeof glob === "string")) fail(where, "is not a list of globs");
  return value.map((written: string) => {
    const glob = written.replace(/^(?:\.\/)+/u, "");
    const source = globSource(glob.replace(/^!{1,2}/u, ""));
    if (source === undefined) fail(where, `holds ${JSON.stringify(written)}, which is not a Biome glob`);
    return { negated: glob.startsWith("!"), pattern: new RegExp(`^${source}$`, "u") };
  });
}

/** Whether `path` matches `globs`: the last glob matching it decides, `fallback` where none does (`matches_with_exceptions_or`). */
function matchesGlobs(globs: Globs, path: string, fallback: boolean): boolean {
  const last = globs.findLast(glob => glob.pattern.test(path));
  return last === undefined ? fallback : !last.negated;
}

/** Whether `globs` hold file `path`: Biome visits no folder holding it that they leave out (`is_dir_included`), and the file matches (`is_file_included`). */
function holdsFile(globs: Globs, path: string): boolean {
  const segments = path.split("/");
  const folders = segments.slice(1).map((_, index) => segments.slice(0, index + 1).join("/"));
  return folders.every(folder => matchesGlobs(globs, folder, true)) && matchesGlobs(globs, path, false);
}

/** An override's `includes` as whole folders (`""`: every file); undefined when one is not `<folder>/**` or `**`. */
function overrideFolders(includes: readonly string[]): string[] | undefined {
  const folders = includes.map(glob => (EVERY_FILE.has(glob) ? "" : folderOfGlob(glob)));
  return folders.every(folder => folder !== undefined) ? folders as string[] : undefined;
}

/** `overrides[index]` of `file` with a `linter` section and `includes` holding any file (`OverrideSettingPattern::is_file_included`). */
function override(value: unknown, file: string, index: number): Override[] {
  const path = `overrides[${index}]`;
  const object = objectAt(value, path);
  if (object.linter === undefined || object.includes === undefined) return [];
  const layer = linterLayer(object.linter, file, `${path}.linter`);
  const includes = globsAt(object.includes, `${path}.includes`);
  if (includes.every(glob => glob.negated)) return [];
  return [{ where: `${file}: ${path}`, includes, folders: overrideFolders(object.includes as string[]), layer }];
}

/** `files.includes` and `linter.includes` of `config`, where set. */
function filtersOf(config: Record<string, unknown>, file: string): Filters {
  const filters: Filters = {};
  for (const key of FILTER_KEYS) {
    const section = config[key];
    const where = `${key}.includes`;
    if (isObject(section) && section.includes !== undefined) filters[key] = { label: `${file}: ${where}`, globs: globsAt(section.includes, where) };
  }
  return filters;
}

/** `extends`: a list of config files, or `"//"` (the root config of a nested one), as `Extends` reads it (crates/biome_configuration/src/extends.rs). */
function extendsOf(value: unknown): { list: string[]; root: boolean } {
  if (value === undefined) return { list: [], root: false };
  if (value === "//") return { list: [], root: true };
  if (!Array.isArray(value) || !value.every(entry => typeof entry === "string")) fail("extends", "is not \"//\" or a list of paths");
  return { list: value, root: false };
}

/** Config file `file`'s object `config`; `root` as `Configuration::is_root` (crates/biome_configuration/src/lib.rs l.361-366). */
function configFile(config: Record<string, unknown>, file: string): ConfigFile {
  const extended = extendsOf(config.extends);
  const root = config.root === undefined ? !extended.root : flag(config.root, "root");
  const overrides = config.overrides ?? [];
  if (!Array.isArray(overrides)) fail("overrides", "is not a list");
  return {
    root,
    extendsRoot: extended.root,
    extends: extended.list,
    linter: config.linter === undefined ? undefined : linterLayer(config.linter, file, "linter"),
    overrides: overrides.flatMap((entry: unknown, index) => override(entry, file, index)),
    filters: filtersOf(config, file),
  };
}

/** `text` of config file `name` as an object: a `.json` file as plain JSON, as Biome reads it; any other as JSONC. */
function parseConfig(name: string, text: string): Record<string, unknown> {
  let parsed = parseJsonc(text);
  try {
    if (name.endsWith(".json")) parsed = JSON.parse(text.replace(/^\uFEFF/u, ""));
  } catch {
    fail("it", "is not plain JSON (Biome reads a .json config without comments or trailing commas)");
  }
  return objectAt(parsed, "it");
}

/** Config file `name` as `read`, if Rerail reads it; a note saying why not otherwise. */
function configOf(name: string, read: FileRead, notes: LinterNote[]): ConfigFile | undefined {
  if (!read.present || read.text === undefined) {
    notes.push({ linter: "biome", text: `${name} is not read: ${read.present ? NOT_A_FILE : "it is not in the repository."}` });
    return undefined;
  }
  const text = read.text;
  const parsed = readFormat(() => configFile(parseConfig(name, text), name));
  if (parsed.ok) return parsed.value;
  notes.push({ linter: "biome", text: `${name} is not read: ${parsed.why}.` });
  return undefined;
}

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
    const path = extendedPath(root, base, entry);
    if (path === undefined) notes.push({ linter: "biome", text: `${file}: extends ${JSON.stringify(entry)} is not read: it is a package or a file outside the repository; the rules it may set keep the pack's values.` });
    const extended = path === undefined ? undefined : configOf(path, readRootFile(root, path), notes);
    if (extended === undefined) unread = `${file} extends ${JSON.stringify(entry)}, which Rerail does not read`;
    layers.push(extended === undefined ? UNREAD : extended.linter ?? NO_LINTER);
    overrides.push(...extended?.overrides ?? []);
    filters.push(extended?.filters ?? {});
  }
  return { layers, overrides, filters, unread };
}

/**
 * The filters configs merged in order give: each key's lists concatenated, as Biome merges a list. An extended config
 * Rerail does not read is taken to set none.
 */
function mergedFilters(merged: readonly Filters[]): Filter[] {
  return FILTER_KEYS.flatMap(key => {
    const set = merged.flatMap(filters => filters[key] ?? []);
    return set.length === 0 ? [] : [{ label: set.map(filter => filter.label).join(" and "), globs: set.flatMap(filter => filter.globs) }];
  });
}

/** A Biome rule as its config knows it, from its metadata (`lint_group_structs!` takes the last analyzer's declaration). */
interface BiomeRule {
  readonly group: string;
  readonly name: string;
  readonly nursery: boolean;
  /** In the recommended preset: recommended and in no domain. */
  readonly presetRecommended: boolean;
  /** In no domain: a group set to a severity turns it on. */
  readonly nonDomain: boolean;
  readonly declarations: readonly BiomeRuleDeclaration[];
}

/** Biome rule `lint/<group>/<rule>`; undefined when Biome 2.5.15 has no such rule. */
function biomeRule(name: string): BiomeRule | undefined {
  const [, group = "", rule = ""] = name.split("/");
  const declarations = BIOME_RULES[`${group}/${rule}`] ?? [];
  const last = declarations.at(-1);
  if (last === undefined) return undefined;
  const [recommended, domains] = last;
  return { group, name: rule, nursery: group === NURSERY, presetRecommended: recommended && domains.length === 0, nonDomain: domains.length === 0, declarations };
}

/** A group as the merged config has it for one rule: a severity, or its preset settings and the rule's entry. */
type GroupState =
  | { readonly kind: "plain"; readonly level: Fact<Level> }
  | { readonly kind: "object"; readonly recommended: Known<boolean>; readonly preset: Known<Preset>; readonly entry: Known<RuleEntry> };

/** The merged config, as far as one rule goes. */
interface Folded {
  /** Whether any `linter` sets `rules` (`recommended_enabled` reads it, crates/biome_service/src/settings.rs l.877-890). */
  readonly rulesSet: boolean | Unread;
  readonly recommended: Known<boolean>;
  readonly preset: Known<Preset>;
  readonly group: GroupState | Unread | undefined;
  readonly domains: ReadonlyMap<string, Fact<DomainValue>>;
  /** An extended config Rerail does not read may set any domain not set after it. */
  readonly domainsUnread: boolean;
  readonly enabled: Known<boolean>;
}

const EMPTY: Folded = { rulesSet: false, recommended: undefined, preset: undefined, group: undefined, domains: new Map(), domainsUnread: false, enabled: undefined };

/** A group object merged over `previous`, as `SeverityOrGroup::merge_with` turns a severity into a group of that severity for every rule. */
function objectBase(previous: GroupState | Unread | undefined): Extract<GroupState, { kind: "object" }> {
  if (previous === undefined) return { kind: "object", recommended: undefined, preset: undefined, entry: undefined };
  if (previous === UNREAD) return { kind: "object", recommended: UNREAD, preset: UNREAD, entry: UNREAD };
  if (previous.kind === "object") return previous;
  return { kind: "object", recommended: undefined, preset: undefined, entry: { value: { level: previous.level.value, options: false }, by: previous.level.by } };
}

/** A rule entry merged over `previous`: a severity alone keeps earlier options (`RuleConfiguration::merge_with`). */
function mergeEntry(previous: Known<RuleEntry>, next: Fact<RuleEntry> | undefined): Known<RuleEntry> {
  if (next === undefined) return previous;
  const options = next.value.options || (isFact(previous) && previous.value.options);
  return { value: { level: next.value.level, options }, by: next.by };
}

/** Group `layer` merged over `previous`, for rule `name`. */
function mergeGroup(previous: GroupState | Unread | undefined, layer: GroupLayer | undefined, name: string): GroupState | Unread | undefined {
  if (layer === undefined) return previous;
  if (layer.kind === "plain") return layer;
  const base = objectBase(previous);
  return { kind: "object", recommended: layer.recommended ?? base.recommended, preset: layer.preset ?? base.preset, entry: mergeEntry(base.entry, layer.rules.get(name)) };
}

/** `rules` merged over `state`, for `rule`. */
function foldRules(state: Folded, rules: RulesLayer | undefined, rule: BiomeRule): Folded {
  if (rules === undefined) return state;
  return {
    ...state,
    rulesSet: true,
    recommended: rules.recommended ?? state.recommended,
    preset: rules.preset ?? state.preset,
    group: mergeGroup(state.group, rules.groups.get(rule.group), rule.name),
  };
}

/**
 * `layer` merged over `state`, as Biome merges an extended config, the config, then the overrides for a file. An
 * extended config Rerail does not read may set any rule, preset and domain; it is taken not to turn the linter off.
 */
function foldLayer(state: Folded, layer: LinterLayer | Unread, rule: BiomeRule): Folded {
  if (layer === UNREAD) {
    return { ...state, rulesSet: state.rulesSet === true || UNREAD, recommended: UNREAD, preset: UNREAD, group: UNREAD, domains: new Map(), domainsUnread: true };
  }
  const merged = foldRules(state, layer.rules, rule);
  return { ...merged, domains: new Map([...merged.domains, ...layer.domains]), enabled: layer.enabled ?? merged.enabled };
}

/** What one part of the config does to a rule: turns it on or off, surely or only possibly (an unread config). */
interface Effect {
  readonly on: boolean;
  readonly sure: boolean;
  readonly explicit: boolean;
  readonly because: string;
  readonly options?: boolean;
}

/** What reading a config for a verdict needs: the file Biome takes, why some of it is not known, and whether the repository has a `package.json`. */
interface Context {
  readonly file: string;
  readonly unread: string;
  readonly manifests: boolean;
}

const unsure = (context: Context): Effect[] => [
  { on: true, sure: false, explicit: true, because: context.unread },
  { on: false, sure: false, explicit: true, because: context.unread },
];

/** The preset `rules` gives every group that does not set its own (`Rules::preset`, rules.rs l.2569-2577). */
function topPreset(folded: Folded, context: Context): Known<Preset> {
  if (isFact(folded.recommended) && !folded.recommended.value) return { value: "none", by: folded.recommended.by };
  if (folded.recommended === UNREAD || folded.preset === UNREAD) return UNREAD;
  return folded.preset ?? { value: "recommended", by: `${context.file}: recommended` };
}

/**
 * A group's own preset, else its parent's (`collect_preset_rules`, crates/biome_configuration_macros/src/group_struct.rs l.294-311).
 * An unread config sets the group's `preset` and `recommended` to unread together, so a group without a preset has a
 * `recommended` that is set or unset, never unread.
 */
function groupPreset(group: Extract<GroupState, { kind: "object" }>, parent: Known<Preset>): Known<Preset> {
  if (group.preset !== undefined) return group.preset;
  if (!isFact(group.recommended)) return parent;
  return { value: group.recommended.value ? "recommended" : "none", by: group.recommended.by };
}

/** What a preset does to `rule`: `all` names every rule; `recommended` holds the recommended rules in no domain. */
function presetEffects(preset: Known<Preset>, rule: BiomeRule, context: Context): Effect[] {
  if (preset === UNREAD) return unsure(context).slice(0, 1);
  if (preset === undefined || preset.value === "none") return [];
  if (preset.value === "all") return [{ on: true, sure: true, explicit: true, because: preset.by }];
  return rule.presetRecommended ? [{ on: true, sure: true, explicit: false, because: preset.by }] : [];
}

/** What a group set to a severity does: on turns on its rules in no domain, off turns off all of them (`SeverityOrGroup`, mod.rs l.1105-1141). */
function plainEffects(level: Fact<Level>, rule: BiomeRule): Effect[] {
  if (level.value === "off") return [{ on: false, sure: true, explicit: true, because: level.by }];
  return rule.nonDomain ? [{ on: true, sure: true, explicit: true, because: level.by }] : [];
}

function entryEffects(entry: Known<RuleEntry>, context: Context): Effect[] {
  if (entry === undefined) return [];
  if (entry === UNREAD) return unsure(context);
  return [{ on: entry.value.level !== "off", sure: true, explicit: true, because: entry.by, options: entry.value.options }];
}

/** What `rules` does to `rule` (`Rules::as_enabled_rules` and `as_disabled_rules`, rules.rs l.2581-2667). */
function ruleEffects(folded: Folded, rule: BiomeRule, context: Context): Effect[] {
  const group = folded.group;
  if (group === UNREAD) return unsure(context);
  if (group === undefined) return rule.nursery ? [] : presetEffects(topPreset(folded, context), rule, context);
  if (group.kind === "plain") return plainEffects(group.level, rule);
  return [...entryEffects(group.entry, context), ...rule.nursery ? [] : presetEffects(groupPreset(group, topPreset(folded, context)), rule, context)];
}

/** Whether a rule's entry turns it on. */
function entryOn(entry: Known<RuleEntry>): boolean | Unread {
  if (entry === UNREAD) return UNREAD;
  return entry !== undefined && entry.value.level !== "off";
}

/** Whether `rules` names the rule on, by itself or its group's severity (`as_explicitly_enabled_rules`), which a domain set to none does not undo. */
function explicitlyOn(folded: Folded, rule: BiomeRule): boolean | Unread {
  const group = folded.group;
  if (group === UNREAD) return UNREAD;
  if (group === undefined) return false;
  if (group.kind === "plain") return group.level.value !== "off" && rule.nonDomain;
  return entryOn(group.entry);
}

function domainValue(folded: Folded, domain: string): Known<DomainValue> {
  return folded.domains.get(domain) ?? (folded.domainsUnread ? UNREAD : undefined);
}

/** What one configured domain does to a rule of it (`record_rule_from_domains`, crates/biome_service/src/file_handlers/mod.rs l.1712-1764). */
function domainEffects(value: Known<DomainValue>, recommended: boolean, context: Context): Effect[] {
  if (value === undefined) return [];
  if (value === UNREAD) return unsure(context);
  if (value.value === "none") return [{ on: false, sure: true, explicit: true, because: value.by }];
  if (value.value === "all") return [{ on: true, sure: true, explicit: true, because: value.by }];
  return recommended ? [{ on: true, sure: true, explicit: false, because: value.by }] : [];
}

/** What the domains do to one analyzer's rule: any domain turning it on wins over one set to none, which a rule named on undoes. */
function declarationDomainEffects(folded: Folded, [recommended, domains]: BiomeRuleDeclaration, named: boolean | Unread, context: Context): Effect[] {
  const effects = domains.flatMap(domain => domainEffects(domainValue(folded, domain), recommended, context));
  const ons = effects.filter(effect => effect.on);
  if (named === true || ons.some(effect => effect.sure)) return ons;
  return [...ons, ...effects.filter(effect => !effect.on).map(effect => (named === UNREAD ? { ...effect, sure: false } : effect))];
}

/**
 * Whether Biome turns on recommended domain rules for the dependencies in `package.json` (`recommended_enabled`). An
 * unread config sets `recommended` and `preset` to unread together, so with `recommended` unset `preset` is not unread.
 */
function recommendedEnabled(folded: Folded): boolean | Unread {
  if (folded.rulesSet !== true) return folded.rulesSet === false || UNREAD;
  if (folded.recommended !== undefined) return folded.recommended === UNREAD ? UNREAD : folded.recommended.value;
  return isFact(folded.preset) && folded.preset.value === "recommended";
}

/**
 * A recommended rule of a domain the config does not set turns on when the nearest `package.json` depends on one of
 * the domain's packages (`ManifestVisitor`, file_handlers/mod.rs l.2002-2063): Rerail does not tell, so it may, in a
 * repository with a `package.json`.
 */
function manifestEffects(folded: Folded, rule: BiomeRule, context: Context): Effect[] {
  if (rule.nursery || !context.manifests || recommendedEnabled(folded) === false) return [];
  const domains = rule.declarations.flatMap(([recommended, ruleDomains]) => (recommended ? ruleDomains : []));
  const open = domains.find(domain => domainValue(folded, domain) === undefined);
  if (open === undefined) return [];
  return [{ on: true, sure: false, explicit: false, because: `${context.file}: Biome turns the recommended ${JSON.stringify(open)} rules on when package.json depends on the domain's packages` }];
}

/** Explicit effects first, so `because` names what the config says about the rule itself. */
const explicitFirst = (effects: readonly Effect[]): Effect[] => [...effects.filter(effect => effect.explicit), ...effects.filter(effect => !effect.explicit)];

/** Where `effects` that turn nothing surely off leave the rule: on when one surely turns it on and nothing may turn it off. */
function onDecision(effects: readonly Effect[], context: Context): PortDecision {
  const ons = explicitFirst(effects.filter(effect => effect.on));
  const unsureOff = effects.find(effect => !effect.on);
  const on = ons.find(effect => effect.sure);
  if (unsureOff !== undefined && ons.length > 0) return { on: "unknown", because: unsureOff.because };
  if (on !== undefined) return { on: true, explicit: on.explicit, because: on.because, ...on.options === true ? { config: { ok: false, reason: OPTIONS_NOT_MAPPED } } : {} };
  if (ons[0] !== undefined) return { on: "unknown", because: ons[0].because };
  return { on: false, explicit: false, because: `${context.file} does not turn it on` };
}

/** The decision `effects` come to: a rule disabled anywhere is off; one enabled anywhere is on; otherwise Biome leaves it off. */
function decide(effects: readonly Effect[], context: Context): PortDecision {
  const off = explicitFirst(effects.filter(effect => !effect.on && effect.sure))[0];
  if (off !== undefined) return { on: false, explicit: off.explicit, because: off.because };
  return onDecision(effects, context);
}

/** What the merged config decides for `rule`. */
function foldedDecision(folded: Folded, rule: BiomeRule, context: Context): PortDecision {
  if (isFact(folded.enabled) && !folded.enabled.value) return { on: false, explicit: false, because: folded.enabled.by };
  const named = explicitlyOn(folded, rule);
  const domainRule = rule.nursery ? [] : rule.declarations.flatMap(declaration => declarationDomainEffects(folded, declaration, named, context));
  return decide([...ruleEffects(folded, rule, context), ...domainRule, ...manifestEffects(folded, rule, context)], context);
}

/** One `linter` section Biome merges for a file; `some` (what it is, for the person) where it may or may not. */
interface Step {
  readonly layer: LinterLayer | Unread;
  readonly some?: string;
}

/** A merged config a file may get, and the last step for only some files it took. */
interface Possible {
  readonly folded: Folded;
  readonly via: string | undefined;
}

/** `possible`, `step` merged over each; a step for only some files keeps each as it was too. Alike merged configs are kept once, the first. */
function foldStep(possible: readonly Possible[], step: Step, rule: BiomeRule): Possible[] {
  const merged = possible.map(state => ({ folded: foldLayer(state.folded, step.layer, rule), via: step.some ?? state.via }));
  if (step.some === undefined) return merged;
  const seen = new Set<string>();
  return [...possible, ...merged].filter(state => {
    const key = JSON.stringify(state.folded, (_, value: unknown) => (value instanceof Map ? [...value] : value === UNREAD ? "unread" : value));
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/**
 * The state `decisions`, each for some files, share: named by the config (explicit) only where each names it, as the
 * files that do not take what names it decide alone. Undefined where their states differ.
 */
function sharedDecision(decisions: readonly PortDecision[]): PortDecision | undefined {
  const [first, ...others] = decisions;
  if (others.some(decision => decision.on !== first!.on)) return undefined;
  return decisions.find(decision => decision.on !== "unknown" && !decision.explicit) ?? first;
}

/** What the merged `steps` decide for `rule`: unknown where a step for only some files may decide it otherwise. */
function ruleDecision(steps: readonly Step[], rule: BiomeRule, context: Context): PortDecision {
  const possible = steps.reduce((states, step) => foldStep(states, step, rule), [{ folded: EMPTY, via: undefined }] as Possible[]);
  const decisions = possible.map(state => foldedDecision(state.folded, rule, context));
  // The first merged config takes no step for only some files; one deciding otherwise takes one.
  const differing = possible[decisions.findIndex(decision => decision.on !== decisions[0]!.on)];
  return sharedDecision(decisions) ?? { on: "unknown", because: `${differing!.via!} is for only some files and may set it` };
}

/** A port's decision from its Biome rules' (all of whose checks it ports, at least one): on when any is on, unknown when any may be. */
function portDecision(decisions: readonly PortDecision[]): PortDecision {
  const on = decisions.filter(decision => decision.on === true);
  return on.find(decision => decision.on === true && decision.explicit)
    ?? on[0]
    ?? decisions.find(decision => decision.on === "unknown")
    ?? decisions.find(decision => decision.on === false && decision.explicit)
    ?? decisions[0]!;
}

/** Rule `name` (`lint/<group>/<rule>`) as Biome 2.5.15 has it; unknown when it has no such rule. */
function nameDecision(steps: readonly Step[], name: string, context: Context): PortDecision {
  const rule = biomeRule(name);
  return rule === undefined ? { on: "unknown", because: `${name} is not a Biome 2.5.15 lint rule` } : ruleDecision(steps, rule, context);
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
