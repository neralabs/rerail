/**
 * One Biome config file, parsed as Biome 2.5.15 parses it and refused where Biome would refuse it. Its `linter` section
 * decides which rules run: `enabled`, `rules` (a preset, `recommended`, groups set to a severity or listing rules) and
 * `domains`; `extends` merges other config files under it and `overrides` merges more `linter` sections over it for the
 * files their `includes` hold; `files.includes` and `linter.includes` leave the files they do not hold unlinted. Each
 * value read keeps where the file sets it, for the person.
 */
import { BIOME_DOMAINS, BIOME_RULES } from "../generated/biome-rules.generated.js";
import { LintConfigFormatError, readFormat } from "../formats/lint-config-formats.js";
import { parseJsonc } from "../formats/jsonc-format.js";
import { NOT_A_FILE, type FileRead, type LinterNote } from "../linter-config.js";
import { globSource, type Globs } from "./biome-globs.js";
import { folderOfGlob } from "./eslint-flat-config-scope.js";

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

export type Level = "off" | "on" | "info" | "warn" | "error";
export type Preset = "recommended" | "all" | "none";
export type DomainValue = "all" | "none" | "recommended";

/** A value the config sets, with where it sets it, for the person. */
export interface Fact<T> {
  readonly value: T;
  readonly by: string;
}

/** A rule's entry in a group: its severity, and whether it sets options. */
export interface RuleEntry {
  readonly level: Level;
  readonly options: boolean;
}

/** A group in one `rules` object: a severity for all its rules, or its own preset and rule entries. */
export type GroupLayer =
  | { readonly kind: "plain"; readonly level: Fact<Level> }
  | { readonly kind: "object"; readonly recommended?: Fact<boolean>; readonly preset?: Fact<Preset>; readonly rules: ReadonlyMap<string, Fact<RuleEntry>> };

/** One `rules` object. */
export interface RulesLayer {
  readonly recommended?: Fact<boolean>;
  readonly preset?: Fact<Preset>;
  readonly groups: ReadonlyMap<string, GroupLayer>;
}

/** One `linter` section Biome merges: a config file's, or an override's. */
export interface LinterLayer {
  readonly enabled?: Fact<boolean>;
  readonly rules?: RulesLayer;
  readonly domains: ReadonlyMap<string, Fact<DomainValue>>;
}

/** An override's `linter` section, for the files its `includes` hold, from the folder of the config it is in. */
export interface Override {
  /** `<file>: overrides[<index>]`, for the person. */
  readonly where: string;
  readonly includes: Globs;
  /** The whole folders `includes` names (`""`: every file), relative to the root; undefined where it holds other files. */
  readonly folders: readonly string[] | undefined;
  readonly layer: LinterLayer;
}

/** A config's `files.includes` or `linter.includes`: Biome lints only the files it holds. */
export interface Filter {
  readonly label: string;
  readonly globs: Globs;
}

/** The keys whose `includes` narrow the files Biome lints, each a list the configs `extends` merges concatenate. */
const FILTER_KEYS = ["files", "linter"] as const;
export type Filters = Partial<Record<(typeof FILTER_KEYS)[number], Filter>>;

/** One config file as read: whether it is a root config, what it extends, its `linter` and its overrides. */
export interface ConfigFile {
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
export function configOf(name: string, read: FileRead, notes: LinterNote[]): ConfigFile | undefined {
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
 * The filters configs merged in order give: each key's lists concatenated, as Biome merges a list. An extended config
 * Rerail does not read is taken to set none.
 */
export function mergedFilters(merged: readonly Filters[]): Filter[] {
  return FILTER_KEYS.flatMap(key => {
    const set = merged.flatMap(filters => filters[key] ?? []);
    return set.length === 0 ? [] : [{ label: set.map(filter => filter.label).join(" and "), globs: set.flatMap(filter => filter.globs) }];
  });
}

