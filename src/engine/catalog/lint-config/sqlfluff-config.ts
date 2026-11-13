/**
 * SQLFluff's rule selection, read as SQLFluff 4.4.0 (`packs/sql/upstream/sqlfluff/gates.yaml`) reads it, as data.
 *
 * In each folder SQLFluff reads `setup.cfg`, `tox.ini`, `pep8.ini`, `.sqlfluff` and `pyproject.toml`, a later one's
 * value replacing an earlier one's (`load_config_at_path`, src/sqlfluff/core/config/loader.py:204): the INI files'
 * `[sqlfluff]` and `[sqlfluff:...]` sections, as configparser reads them with `delimiters="="`, no interpolation and
 * keys kept in their case, each value coerced (`load_ini_string` and `coerce_value`, config/ini.py), and
 * `pyproject.toml`'s `[tool.sqlfluff]` (config/toml.py). For a file it lints, SQLFluff layers the configs of every
 * folder from the working folder (the repository root, where Rerail takes it to run) down to the file's own, a deeper
 * folder's value replacing a shallower one's (`load_config_up_to_path`, loader.py:269). Rerail reads them all: the
 * root's configs decide for every file, and each folder below holding a config decides for its files. The person's
 * own config in their home folder is not read.
 *
 * Two keys select the rules (`get_rulepack`, src/sqlfluff/core/rules/base.py:1190): `rules`, the rules SQLFluff runs
 * (every rule when unset or empty: default_config.cfg `rules = all`), and `exclude_rules`, which wins over it. Each
 * is a comma-separated list of references, a code (`CV05`), a name (`convention.is_null`), a group (`convention`,
 * `core`, `all`) or a legacy alias (`L049`), or an `fnmatch` glob over them, matched in case on POSIX
 * (`_expand_rule_refs`, base.py:1099). A port's `suppress.sqlfluff.rules` gives exactly those references for its
 * rule, so a reference selects the port when it is, or globs, one of them. A few rules report nothing, selected or
 * not, until their own section sets `force_enable` true; SQLFluff stops with an error on a `force_enable` that is not
 * a boolean, `0` or `1` (`_validate_config_options`, base.py:1021).
 */
import { posix } from "node:path";
import { fnmatchPattern } from "./formats/fnmatch.js";
import { readIni, type IniSections } from "./formats/ini-format.js";
import { LintConfigFormatError, readFormat, readTomlTrees, type TomlValue } from "./formats/lint-config-formats.js";
import { inFolder, NOT_A_FILE, readRootFile, type FolderDecision, type LinterConfig, type LinterNote, type LinterReader, type PortDecision, type PortVerdict } from "./linter-config.js";
import { repositoryFiles } from "./repository-files.js";

/** SQLFluff's config files in one folder, a later one's value replacing an earlier one's (loader.py:225-231). */
const SQLFLUFF_FILES = ["setup.cfg", "tox.ini", "pep8.ini", ".sqlfluff", "pyproject.toml"];
const PYPROJECT = "pyproject.toml";
const SQLFLUFF_INI = ".sqlfluff";
const KEYS = ["rules", "exclude_rules"] as const;
type SelectionKey = (typeof KEYS)[number];
/** The group every rule is in: `rules = all` is SQLFluff's default selection and names no rule. */
const EVERY_RULE = "all";
/**
 * The rules the packs port that report nothing until `force_enable` is true in their own section, whatever selects
 * them (default_config.cfg:480-486 sets it false; PG01.py:77 and PG02.py:59 return no result without it). Their
 * section's value replaces a `[sqlfluff:rules]` one (base.py:1307-1312), so only their own section is read.
 */
const FORCE_ENABLED = ["postgres.excessive_locks", "postgres.not_valid_foreign_key"];
/** What `int()` or `float()` reads in `coerce_value`, underscores between digits included. */
const INI_NUMBER = /^[+-]?(?:\d(?:_?\d)*(?:\.(?:\d(?:_?\d)*)?)?|\.\d(?:_?\d)*)(?:e[+-]?\d(?:_?\d)*)?$/iu;
/** The words `coerce_value` reads as other than text, in any case. */
const INI_WORDS: Readonly<Record<string, boolean | null>> = { true: true, false: false, none: null };

/** A setting SQLFluff stops on, or one in a config Rerail does not read. */
interface Unknown {
  readonly unknown: string;
  /** SQLFluff stops on it, so every rule is unknown, not only the one it sets. */
  readonly stops?: true;
}

/** One key's value in one config: its references, empty for the default. */
interface Selection {
  /** Where it is set, for the person: `.sqlfluff`, `pyproject.toml [tool.sqlfluff.core]`. */
  readonly label: string;
  readonly references: readonly string[];
}

/** A `force_enable` one config sets for a rule: whether SQLFluff reads it as true, and where it is set. */
interface ForceEnable {
  readonly on: boolean;
  readonly because: string;
}

/** One config file as read: what it sets, or why it is not read. */
type ConfigLayer =
  | {
    readonly label: string;
    readonly keys: ReadonlyMap<SelectionKey, Selection | Unknown>;
    readonly forced: ReadonlyMap<string, ForceEnable | Unknown>;
    /** The `ignore_paths` patterns discovery takes from it, relative to its folder. */
    readonly ignorePaths?: readonly string[] | undefined;
  }
  | { readonly unread: string; readonly path: string };

/** What one setting comes to over the layers: a value, unset (the default), or not known. */
type Resolved<T> = T | undefined | Unknown;

/** The settings over every layer that decide which rules SQLFluff runs. */
interface SqlfluffSelection {
  readonly rules: Resolved<Selection>;
  readonly exclude: Resolved<Selection>;
  readonly forced: ReadonlyMap<string, Resolved<ForceEnable>>;
  /** The configs read, for the person: `setup.cfg and .sqlfluff`. */
  readonly labels: string;
}

/** `value` as `coerce_value` (config/ini.py:12) reads it: a number, a boolean, `None` (null) or the text itself. */
function coerceIni(value: string): number | boolean | null | string {
  const text = value.trim();
  if (INI_NUMBER.test(text)) return Number(text.replaceAll("_", ""));
  // `float()` reads these too; only that they are no 0 or 1, and true, matters.
  if (/^[+-]?(?:inf|infinity|nan)$/iu.test(text)) return Number.POSITIVE_INFINITY;
  const word = text.toLowerCase();
  return Object.hasOwn(INI_WORDS, word) ? INI_WORDS[word]! : value;
}

/** Whether a coerced value is false to Python: `False`, `None`, zero or empty. */
const pythonFalse = (value: number | boolean | null | string): boolean => value === false || value === null || value === 0 || value === "";

/** The references a comma-separated `text` lists, trimmed as `split_comma_separated_string` does (helpers/string.py:85). */
const commaReferences = (text: string): string[] => text.split(",").map(reference => reference.trim()).filter(reference => reference !== "");

/**
 * INI `value` of `key` as `_handle_comma_separated_values` (fluffconfig.py:144) reads it once coerced: a false one is
 * the default, a text one is split; SQLFluff stops on any other.
 */
function iniReferences(label: string, key: SelectionKey, value: string): Selection | Unknown {
  const coerced = coerceIni(value);
  if (pythonFalse(coerced)) return { label, references: [] };
  if (typeof coerced === "string") return { label, references: commaReferences(coerced) };
  return { unknown: `${label}: SQLFluff stops with an error on ${key} = ${value.trim()}, which is not a list` };
}

/**
 * The `force_enable` coerced `value` sets, where it is set: SQLFluff stops on one other than a boolean, `None`, `0` or
 * `1`, when it selects the rule; Rerail does not tell when that is, so every rule is then unknown.
 */
function forceEnable(value: number | boolean | null | string, because: string): ForceEnable | Unknown {
  if (value === null || typeof value === "boolean" || value === 0 || value === 1) return { on: value === true || value === 1, because };
  return { unknown: `${because}: SQLFluff stops with an error where it selects the rule, as force_enable must be True or False`, stops: true };
}

/** The `force_enable` each force-enabled rule's own section in `sections` sets, as configparser and `coerce_value` read it. */
function iniForced(sections: IniSections, label: string): Map<string, ForceEnable | Unknown> {
  return new Map(FORCE_ENABLED.flatMap(rule => {
    const value = sections.get(`sqlfluff:rules:${rule}`)?.get("force_enable");
    return value === undefined ? [] : [[rule, forceEnable(coerceIni(value), `${label} [sqlfluff:rules:${rule}]: force_enable = ${value}`)] as const];
  }));
}

/**
 * `.sqlfluff`'s `ignore_paths` once coerced, as discovery reads it (`_load_configfile`, linter/discovery.py:76): text
 * split at commas, nothing kept off the ends; anything else ignores nothing. Other INI files' are not read by SQLFluff.
 */
function iniIgnorePaths(label: string, core: ReadonlyMap<string, string> | undefined): string[] | undefined {
  const value = core?.get("ignore_paths");
  if (value === undefined || posix.basename(label) !== SQLFLUFF_INI) return undefined;
  const coerced = coerceIni(value);
  return typeof coerced === "string" && coerced !== "" ? coerced.split(",") : undefined;
}

/** INI file `label` as a layer, or undefined when it has no SQLFluff section. Throws `LintConfigFormatError`. */
function iniLayer(text: string, label: string): ConfigLayer | undefined {
  const sections = readIni(text, { delimiters: ["="], lowerCaseOptions: false, interpolation: false });
  const own = [...sections.keys()].some(section => section === "sqlfluff" || section.startsWith("sqlfluff:"));
  if (!own && posix.basename(label) !== SQLFLUFF_INI) return undefined;
  const core = sections.get("sqlfluff");
  const keys = new Map(KEYS.flatMap(key => {
    const value = core?.get(key);
    return value === undefined ? [] : [[key, iniReferences(label, key, value)] as const];
  }));
  return { label, keys, forced: iniForced(sections, label), ignorePaths: iniIgnorePaths(label, core) };
}

/** A TOML value of `rules` or `exclude_rules`: a comma-separated string or a list of strings, kept as they are (toml.py). */
function tomlReferences(value: TomlValue, key: SelectionKey): string[] {
  if (typeof value === "string") return commaReferences(value);
  if (Array.isArray(value) && value.every(item => typeof item === "string")) return value as string[];
  throw new LintConfigFormatError(`${key} in [tool.sqlfluff.core] is not a string or a list of strings`);
}

/** A TOML boolean or number as Python holds it: `true`, `false`, or the number (NaN for a date or time). */
const tomlScalar = (scalar: string): boolean | number => scalar === "true" || (scalar !== "false" && Number(scalar.replaceAll("_", "")));

/** The `force_enable` TOML `value` sets for rule `rule` in file `label`: TOML strings are not coerced, so SQLFluff stops on one. */
function tomlForceEnable(value: TomlValue, rule: string, label: string): ForceEnable | Unknown {
  if (Array.isArray(value) || value instanceof Map) throw new LintConfigFormatError(`force_enable of ${rule} in [tool.sqlfluff.rules] is not a boolean`);
  const where = `${label} [tool.sqlfluff.rules.${JSON.stringify(rule)}]: force_enable =`;
  if (typeof value === "string") return forceEnable(value, `${where} ${JSON.stringify(value)}`);
  return forceEnable(tomlScalar(value.scalar), `${where} ${value.scalar}`);
}

/** Rule `rule`'s own table under `[tool.sqlfluff.rules]`: `."postgres.excessive_locks"` or `.postgres.excessive_locks` (toml.py `_condense_rule_record`). */
function tomlRuleTable(rules: ReadonlyMap<string, TomlValue> | undefined, rule: string): ReadonlyMap<string, TomlValue> | undefined {
  const quoted = rules?.get(rule);
  if (quoted instanceof Map) return quoted;
  let table: TomlValue | ReadonlyMap<string, TomlValue> | undefined = rules;
  for (const part of rule.split(".")) table = table instanceof Map ? table.get(part) : undefined;
  return table instanceof Map ? table : undefined;
}

/** The `force_enable` each force-enabled rule's own table under `[tool.sqlfluff.rules]` of file `label` sets. */
function tomlForced(rules: ReadonlyMap<string, TomlValue> | undefined, label: string): Map<string, ForceEnable | Unknown> {
  return new Map(FORCE_ENABLED.flatMap(rule => {
    const value = tomlRuleTable(rules, rule)?.get("force_enable");
    return value === undefined ? [] : [[rule, tomlForceEnable(value, rule, label)] as const];
  }));
}

/** `ignore_paths` of `[tool.sqlfluff.core]` as discovery reads it: a string split at commas, or a list of strings; empty, nothing. */
function tomlIgnorePaths(value: TomlValue | undefined): string[] | undefined {
  if (value === undefined) return undefined;
  const paths = typeof value === "string" ? value.split(",") : value;
  if (!Array.isArray(paths) || !paths.every(item => typeof item === "string")) throw new LintConfigFormatError("ignore_paths in [tool.sqlfluff.core] is not a string or a list of strings");
  return paths.length === 0 ? undefined : (paths as string[]);
}

/** A `pyproject.toml`'s `[tool.sqlfluff]` as a layer, or undefined when it has none. Throws `LintConfigFormatError`. */
function tomlLayer(text: string, label: string): ConfigLayer | undefined {
  const found = readTomlTrees(text, ["tool.sqlfluff.core", "tool.sqlfluff.rules"]);
  if (!found.paths.some(path => path === "tool.sqlfluff" || path.startsWith("tool.sqlfluff."))) return undefined;
  const core = found.trees.get("tool.sqlfluff.core");
  const keys = new Map(KEYS.flatMap(key => {
    const value = core?.get(key);
    return value === undefined ? [] : [[key, { label: `${label} [tool.sqlfluff.core]`, references: tomlReferences(value, key) }] as const];
  }));
  return { label, keys, forced: tomlForced(found.trees.get("tool.sqlfluff.rules"), label), ignorePaths: tomlIgnorePaths(core?.get("ignore_paths")) };
}

/** A layer for file `name`, not read for `why`, with a note saying so. */
function unreadLayer(name: string, why: string, notes: LinterNote[]): ConfigLayer {
  notes.push({ linter: "sqlfluff", text: `${name} is not read: ${why}` });
  return { unread: `${name} is not read`, path: name };
}

/** Config file `path` (relative to the root) as a layer; undefined when it is absent or holds no SQLFluff config. */
function readLayer(root: string, path: string, notes: LinterNote[]): ConfigLayer | undefined {
  const read = readRootFile(root, path);
  if (!read.present) return undefined;
  const text = read.text;
  if (text === undefined) return unreadLayer(path, NOT_A_FILE, notes);
  const name = posix.basename(path);
  const layer = readFormat(() => (name === PYPROJECT ? tomlLayer(text, path) : iniLayer(text, path)));
  if (layer.ok) return layer.value;
  // A shared file that may hold no SQLFluff config at all is not worth a note.
  return name !== SQLFLUFF_INI && !text.includes("sqlfluff") ? undefined : unreadLayer(path, `${layer.why}.`, notes);
}

/** The folders holding SQLFluff configs, outermost first (the root, `""`, before all), each with its layers; a folder whose files configure nothing is left out. */
function configFolders(root: string, notes: LinterNote[]): Map<string, ConfigLayer[]> {
  const nested = repositoryFiles(root).filter(path => path.includes("/") && SQLFLUFF_FILES.includes(posix.basename(path)));
  const byFolder = Map.groupBy([...SQLFLUFF_FILES, ...nested], path => (path.includes("/") ? posix.dirname(path) : ""));
  const folders = new Map<string, ConfigLayer[]>();
  // The sort is stable, so the root, grouped first, stays ahead of the folders at the top.
  for (const [folder, paths] of [...byFolder].sort(([left], [right]) => left.split("/").length - right.split("/").length)) {
    const ordered = paths.toSorted((left, right) => SQLFLUFF_FILES.indexOf(posix.basename(left)) - SQLFLUFF_FILES.indexOf(posix.basename(right)));
    const layers = ordered.flatMap(path => readLayer(root, path, notes) ?? []);
    if (layers.length > 0) folders.set(folder, layers);
  }
  return folders;
}

/** What a setting comes to: its value in the last layer that sets it, unknown where a layer after that is not read. */
function resolve<T>(layers: readonly ConfigLayer[], valueIn: (layer: Exclude<ConfigLayer, { unread: string }>) => T | Unknown | undefined): Resolved<T> {
  for (const layer of [...layers].reverse()) {
    if ("unread" in layer) return { unknown: layer.unread };
    const value = valueIn(layer);
    if (value !== undefined) return value;
  }
  return undefined;
}

/** SQLFluff's selection over `layers`, shallowest first; `SQLFluff's defaults` names an empty one. */
function selectionOf(layers: readonly ConfigLayer[]): SqlfluffSelection {
  const labels = layers.flatMap(layer => ("label" in layer ? [layer.label] : []));
  return {
    rules: resolve(layers, layer => layer.keys.get("rules")),
    exclude: resolve(layers, layer => layer.keys.get("exclude_rules")),
    forced: new Map(FORCE_ENABLED.map(rule => [rule, resolve(layers, layer => layer.forced.get(rule))])),
    labels: labels.length === 0 ? "SQLFluff's defaults" : labels.join(" and "),
  };
}

/** The first of `selection`'s references that selects a rule with references `names`. */
function selecting(selection: Selection, names: readonly string[]): string | undefined {
  return selection.references.find(reference => {
    const pattern = fnmatchPattern(reference);
    return names.some(name => pattern.test(name));
  });
}

/** Whether `resolved` is not known: SQLFluff stops on it, or a config Rerail does not read may set it. */
const isUnknown = <T>(resolved: Resolved<T>): resolved is Unknown => typeof resolved === "object" && resolved !== null && "unknown" in resolved;

/** The explicit off `exclude_rules` gives a rule with references `names`, if it excludes it. */
function excludedBy(exclude: Resolved<Selection>, names: readonly string[]): PortDecision | undefined {
  if (exclude === undefined || isUnknown(exclude)) return undefined;
  const reference = selecting(exclude, names);
  return reference === undefined ? undefined : { on: false, explicit: true, because: `${exclude.label}: ${JSON.stringify(reference)} in exclude_rules` };
}

/** Whether `rules` (unset: every rule) selects a rule with references `names`, and why. */
function selectedBy(rules: Selection | undefined, names: readonly string[], labels: string): Extract<PortDecision, { on: boolean }> {
  if (rules === undefined || rules.references.length === 0) return { on: true, explicit: false, because: `${rules?.label ?? labels}: no rules set, so SQLFluff runs every rule` };
  const reference = selecting(rules, names);
  if (reference === undefined) return { on: false, explicit: false, because: `${rules.label}: rules does not select it` };
  return { on: true, explicit: reference !== EVERY_RULE, because: `${rules.label}: ${JSON.stringify(reference)} in rules` };
}

/** A selected rule with references `names` as `force_enable` leaves it: on only where its own section sets it true. */
function forcedBy(selection: SqlfluffSelection, names: readonly string[], selected: PortDecision): PortDecision {
  const rule = FORCE_ENABLED.find(name => names.includes(name));
  if (rule === undefined) return selected;
  const forced = selection.forced.get(rule);
  if (isUnknown(forced)) return { on: "unknown", because: forced.unknown };
  if (forced !== undefined) return { on: forced.on, explicit: true, because: forced.because };
  return { on: false, explicit: false, because: `${selection.labels}: no force_enable = True for ${rule}, without which SQLFluff reports nothing for it` };
}

/** What SQLFluff's selection decides for a rule with references `names`: `exclude_rules` wins over `rules`. */
function decide(selection: SqlfluffSelection, names: readonly string[]): PortDecision {
  const excluded = excludedBy(selection.exclude, names);
  if (excluded !== undefined) return excluded;
  // A `force_enable` SQLFluff may stop on leaves every rule unknown: it stops wherever it selects that rule.
  const unknown = isUnknown(selection.rules) ? selection.rules : [...selection.forced.values()].find(forced => isUnknown(forced) && forced.stops === true);
  if (isUnknown(unknown)) return { on: "unknown", because: unknown.unknown };
  const selected = selectedBy(selection.rules as Selection | undefined, names, selection.labels);
  if (!selected.on) return selected;
  // A config not read may exclude what the rules select; it cannot select what they do not.
  if (isUnknown(selection.exclude)) return { on: "unknown", because: selection.exclude.unknown };
  return forcedBy(selection, names, selected);
}

/** Whether `path` (relative to the root) is in folder `folder`, `""` being the root. */
const within = (path: string, folder: string): boolean => folder === "" || path === folder || path.startsWith(`${folder}/`);

/** Ignore patterns SQLFluff reads in folder `folder` (`.sqlfluffignore`, or `ignore_paths`), and where they are written. */
interface IgnoreSpec {
  readonly folder: string;
  readonly label: string;
  /** Its lines; undefined when the file is not read. */
  readonly lines: readonly string[] | undefined;
  /** Why it is not read, when its lines are not. */
  readonly why?: string;
}

/** A gitignore pattern naming a path literally, relative to its spec's folder. */
interface LiteralPattern {
  readonly line: string;
  readonly path: string;
  /** It matches only there, not at any depth: it has a `/` before its end and does not start with `**` `/`. */
  readonly anchored: boolean;
  /** It ends in `/` or `/**`, so it matches no file of that name. */
  readonly foldersOnly: boolean;
}

/** Gitignore `line` (pathspec's `gitignore` patterns) as a literal pattern; undefined for a glob, a negation or an escape. */
function literalPattern(line: string): LiteralPattern | undefined {
  const text = line.replace(/ +$/u, "");
  const path = text.replace(/^\*\*\//u, "").replace(/(?:\/\*\*|\/)$/u, "").replace(/^\//u, "");
  if (text.startsWith("!") || /[*?[\\]/u.test(path) || path.split("/").some(part => part === "" || part === "." || part === "..")) return undefined;
  return { line, path, anchored: !text.startsWith("**/") && text.replace(/\/$/u, "").includes("/"), foldersOnly: /\/(?:\*\*)?$/u.test(text) };
}

/** Whether `pattern` matches `path` (relative to the root) under its spec's folder: there, or unanchored, at any depth. */
function matchesPath(spec: IgnoreSpec, pattern: LiteralPattern, path: string): boolean {
  const inSpec = spec.folder === "" ? path : path.slice(spec.folder.length + 1);
  return path !== spec.folder && within(path, spec.folder) && (inSpec === pattern.path || (!pattern.anchored && inSpec.endsWith(`/${pattern.path}`)));
}

/** The ignore specs discovery reads: every `.sqlfluffignore` (linter/discovery.py:110), and the `ignore_paths` of `.sqlfluff` and `pyproject.toml`. */
function ignoreSpecs(root: string, folders: ReadonlyMap<string, readonly ConfigLayer[]>): IgnoreSpec[] {
  const names = new Set([".sqlfluffignore", ...repositoryFiles(root).filter(path => posix.basename(path) === ".sqlfluffignore")]);
  const files = [...names].flatMap(path => {
    const read = readRootFile(root, path);
    return read.present ? [{ folder: path.includes("/") ? posix.dirname(path) : "", label: path, lines: read.text?.split(/\r\n|\r|\n/u), why: NOT_A_FILE }] : [];
  });
  const configs = [...folders].flatMap(([folder, layers]) => layers.flatMap(layer => layerIgnoreSpec(folder, layer) ?? []));
  return [...files, ...configs];
}

/**
 * The ignore spec config `layer` in `folder` gives: its `ignore_paths`, or, when it is a `.sqlfluff` or
 * `pyproject.toml` Rerail does not read (noted already), one not read for every file: discovery reads those wherever
 * SQLFluff lints, and stops on every file where it cannot.
 */
function layerIgnoreSpec(folder: string, layer: ConfigLayer): IgnoreSpec | undefined {
  if (!("unread" in layer)) return layer.ignorePaths === undefined ? undefined : { folder, label: `${layer.label}: ignore_paths`, lines: layer.ignorePaths };
  return [SQLFLUFF_INI, PYPROJECT].includes(posix.basename(layer.path)) ? { folder: "", label: layer.path, lines: undefined } : undefined;
}

/** `spec`'s patterns, each naming a path literally; undefined, with a note, when Rerail cannot read them all. */
function specPatterns(spec: IgnoreSpec, notes: LinterNote[]): LiteralPattern[] | undefined {
  const lines = (spec.lines ?? []).filter(line => line.trim() !== "" && !line.startsWith("#"));
  const patterns = lines.map(line => literalPattern(line));
  const glob = lines.find((_, index) => patterns[index] === undefined);
  if (spec.lines !== undefined && glob === undefined) return patterns as LiteralPattern[];
  const why = glob === undefined ? spec.why : `${JSON.stringify(glob)} names no path literally, and Rerail reads only those.`;
  if (why !== undefined) notes.push({ linter: "sqlfluff", text: `${spec.label} is not read: ${why} The rules keep the pack's values where it applies.` });
  return undefined;
}

/** The folders SQLFluff ignores (or may), from the repository's listing; the SQL files it ignores outside them go to `notes`. */
function readIgnores(root: string, folders: ReadonlyMap<string, readonly ConfigLayer[]>, notes: LinterNote[]): { specs: number; ignored: FolderDecision[] } {
  const specs = ignoreSpecs(root, folders).map(spec => ({ spec, patterns: specPatterns(spec, notes) }));
  const files = repositoryFiles(root);
  const listed = [...new Set(files.flatMap(file => file.split("/").slice(0, -1).map((_, index, parts) => parts.slice(0, index + 1).join("/"))))];
  const ignored = specs.flatMap(({ spec, patterns }): FolderDecision[] => {
    if (patterns === undefined) return [{ folder: spec.folder, on: "unknown", because: `${spec.label} is not read` }];
    return patterns.flatMap(pattern => listed.filter(folder => matchesPath(spec, pattern, folder)).map(folder => ({ folder, on: false, explicit: true, because: `${spec.label}: ${JSON.stringify(pattern.line)}` })));
  });
  const rest = files.filter(file => file.endsWith(".sql") && !ignored.some(decision => within(file, decision.folder)));
  for (const { spec, patterns } of specs) {
    const skipped = rest.filter(file => (patterns ?? []).some(pattern => !pattern.foldersOnly && matchesPath(spec, pattern, file)));
    if (skipped.length > 0) notes.push({ linter: "sqlfluff", text: `${spec.label}: SQLFluff ignores ${skipped.join(", ")}, which Rerail cannot set rules for alone (it sets them for whole folders), so the rules stay on there.` });
  }
  return { specs: specs.length, ignored };
}

export const sqlfluffReader: LinterReader = {
  linter: "sqlfluff",
  kinds: ["sqlfluff"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const folders = configFolders(root, notes);
    const { specs, ignored } = readIgnores(root, folders, notes);
    if (folders.size === 0 && specs === 0) return undefined;
    // Each folder's selection layers the configs of every folder from the root down to it, outermost first. One
    // inside a folder SQLFluff ignores (or may) decides nothing there.
    const selections = new Map([...folders.keys()].map(folder => [folder, selectionOf([...folders].filter(([outer]) => within(folder, outer)).flatMap(([, layers]) => layers))]));
    const whole = selections.get("") ?? selectionOf([]);
    const nested = [...selections].filter(([folder]) => folder !== "" && !ignored.some(decision => within(folder, decision.folder)));
    const atRoot = ignored.find(decision => decision.folder === "");
    return {
      verdict(port): PortVerdict | undefined {
        const names = port.suppress.sqlfluff;
        if (names.length === 0) return undefined;
        const configured = nested.map(([folder, selection]) => inFolder(folder, decide(selection, names)));
        const decision: PortDecision = atRoot === undefined ? decide(whole, names) : { on: "unknown", because: atRoot.because };
        return { decision, folders: [...configured, ...ignored.filter(folder => folder.folder !== "")] };
      },
    };
  },
};
