/**
 * SQLFluff's rule selection, read as SQLFluff 4.4.0 (`packs/sql/upstream/sqlfluff/gates.yaml`) reads it, as data.
 *
 * In each folder SQLFluff reads `setup.cfg`, `tox.ini`, `pep8.ini`, `.sqlfluff` and `pyproject.toml`, a later one's
 * value replacing an earlier one's (`load_config_at_path`, src/sqlfluff/core/config/loader.py:204): the INI files'
 * `[sqlfluff]` and `[sqlfluff:...]` sections, as configparser reads them with `delimiters="="`, no interpolation and
 * keys kept in their case (`load_ini_string`, config/ini.py), and `pyproject.toml`'s `[tool.sqlfluff]` (config/toml.py).
 * Rerail reads them at the repository root; a `.sqlfluff` in a folder below, which SQLFluff layers on top for the
 * files under it, and the person's own config in their home folder are not read.
 *
 * Two keys select the rules (`get_rulepack`, src/sqlfluff/core/rules/base.py:1190): `rules`, the rules SQLFluff runs
 * (every rule when unset or empty: default_config.cfg `rules = all`), and `exclude_rules`, which wins over it. Each
 * is a comma-separated list of references, a code (`CV05`), a name (`convention.is_null`), a group (`convention`,
 * `core`, `all`) or a legacy alias (`L049`), or an `fnmatch` glob over them (`_expand_rule_refs`, base.py:1099). A
 * port's `suppress.sqlfluff.rules` gives exactly those references for its rule, so a reference selects the port when
 * it is, or globs, one of them. A few rules report nothing, selected or not, until their own section sets
 * `force_enable = True`.
 */
import { readIni, type IniSections } from "./formats/ini-format.js";
import { LintConfigFormatError, readFormat, readTomlTrees, type TomlValue } from "./formats/lint-config-formats.js";
import { NOT_A_FILE, readRootFile, type LinterConfig, type LinterNote, type LinterReader, type PortDecision, type PortVerdict } from "./linter-config.js";

/** SQLFluff's config files in one folder, a later one's value replacing an earlier one's (loader.py:225-231). */
const SQLFLUFF_FILES = ["setup.cfg", "tox.ini", "pep8.ini", ".sqlfluff", "pyproject.toml"];
const PYPROJECT = "pyproject.toml";
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
  | { readonly label: string; readonly keys: ReadonlyMap<SelectionKey, Selection>; readonly forced: ReadonlyMap<string, ForceEnable> }
  | { readonly unread: string };

/** What one setting comes to over the layers: a value, unset (the default), or not known. */
type Resolved<T> = T | undefined | { readonly unknown: string };

/** The settings over every layer that decide which rules SQLFluff runs. */
interface SqlfluffSelection {
  readonly rules: Resolved<Selection>;
  readonly exclude: Resolved<Selection>;
  readonly forced: ReadonlyMap<string, Resolved<ForceEnable>>;
  /** The configs read, for the person: `setup.cfg and .sqlfluff`. */
  readonly labels: string;
}

/** Whether `coerce_value` (config/ini.py:12) makes `value` falsy: empty, `None`, `False` or a zero number. */
function iniFalsy(value: string): boolean {
  const text = value.trim();
  if (text === "" || /^(?:none|false)$/iu.test(text)) return true;
  return /^[+-]?[\d_.]+(?:e[+-]?\d+)?$/iu.test(text) && /\d/u.test(text) && Number(text.replaceAll("_", "")) === 0;
}

/** The references `value` lists, split at commas and trimmed as `split_comma_separated_string` does (helpers/string.py:85). */
const commaReferences = (value: string): string[] => (iniFalsy(value) ? [] : value.split(",").map(reference => reference.trim()).filter(reference => reference !== ""));

/** The keys `referencesOf` gives references for, each as set in the file known as `label`. */
function selections(label: string, referencesOf: (key: SelectionKey) => readonly string[] | undefined): Map<SelectionKey, Selection> {
  return new Map(KEYS.flatMap(key => {
    const references = referencesOf(key);
    return references === undefined ? [] : [[key, { label, references }] as const];
  }));
}

/** The `force_enable` each force-enabled rule's own section in `sections` sets, as configparser and `coerce_value` read it. */
function iniForced(sections: IniSections, name: string): Map<string, ForceEnable> {
  return new Map(FORCE_ENABLED.flatMap(rule => {
    const value = sections.get(`sqlfluff:rules:${rule}`)?.get("force_enable");
    return value === undefined ? [] : [[rule, { on: !iniFalsy(value), because: `${name} [sqlfluff:rules:${rule}]: force_enable = ${value}` }] as const];
  }));
}

/** INI file `name` as a layer, or undefined when it has no SQLFluff section. Throws `LintConfigFormatError`. */
function iniLayer(text: string, name: string): ConfigLayer | undefined {
  const sections = readIni(text, { delimiters: ["="], lowerCaseOptions: false, interpolation: false });
  const own = [...sections.keys()].some(section => section === "sqlfluff" || section.startsWith("sqlfluff:"));
  if (!own && name !== ".sqlfluff") return undefined;
  const core = sections.get("sqlfluff");
  const keys = selections(name, key => {
    const value = core?.get(key);
    return value === undefined ? undefined : commaReferences(value);
  });
  return { label: name, keys, forced: iniForced(sections, name) };
}

/** A TOML value of `rules` or `exclude_rules`: a comma-separated string or a list of strings, kept as they are (toml.py). */
function tomlReferences(value: TomlValue, key: SelectionKey): string[] {
  if (typeof value === "string") return commaReferences(value);
  if (Array.isArray(value) && value.every(item => typeof item === "string")) return value as string[];
  throw new LintConfigFormatError(`${key} in [tool.sqlfluff.core] is not a string or a list of strings`);
}

/**
 * The `force_enable` TOML `value` sets for rule `rule`, as Python reads it: a non-empty string (TOML strings are not
 * coerced), `true` or a non-zero number is true.
 */
function tomlForceEnable(value: TomlValue, rule: string): ForceEnable {
  const where = `${PYPROJECT} [tool.sqlfluff.rules.${JSON.stringify(rule)}]: force_enable =`;
  if (typeof value === "string") return { on: value !== "", because: `${where} ${JSON.stringify(value)}` };
  if (Array.isArray(value) || value instanceof Map) throw new LintConfigFormatError(`force_enable of ${rule} in [tool.sqlfluff.rules] is not a boolean`);
  return { on: value.scalar !== "false" && Number(value.scalar.replaceAll("_", "")) !== 0, because: `${where} ${value.scalar}` };
}

/** Rule `rule`'s own table under `[tool.sqlfluff.rules]`: `."postgres.excessive_locks"` or `.postgres.excessive_locks` (toml.py `_condense_rule_record`). */
function tomlRuleTable(rules: ReadonlyMap<string, TomlValue> | undefined, rule: string): ReadonlyMap<string, TomlValue> | undefined {
  const quoted = rules?.get(rule);
  if (quoted instanceof Map) return quoted;
  let table: TomlValue | ReadonlyMap<string, TomlValue> | undefined = rules;
  for (const part of rule.split(".")) table = table instanceof Map ? table.get(part) : undefined;
  return table instanceof Map ? table : undefined;
}

/** The `force_enable` each force-enabled rule's own table under `[tool.sqlfluff.rules]` sets. */
function tomlForced(rules: ReadonlyMap<string, TomlValue> | undefined): Map<string, ForceEnable> {
  return new Map(FORCE_ENABLED.flatMap(rule => {
    const value = tomlRuleTable(rules, rule)?.get("force_enable");
    return value === undefined ? [] : [[rule, tomlForceEnable(value, rule)] as const];
  }));
}

/** `pyproject.toml`'s `[tool.sqlfluff]` as a layer, or undefined when it has none. Throws `LintConfigFormatError`. */
function tomlLayer(text: string): ConfigLayer | undefined {
  const found = readTomlTrees(text, ["tool.sqlfluff.core", "tool.sqlfluff.rules"]);
  if (!found.paths.some(path => path === "tool.sqlfluff" || path.startsWith("tool.sqlfluff."))) return undefined;
  const core = found.trees.get("tool.sqlfluff.core");
  const keys = selections(`${PYPROJECT} [tool.sqlfluff.core]`, key => {
    const value = core?.get(key);
    return value === undefined ? undefined : tomlReferences(value, key);
  });
  return { label: PYPROJECT, keys, forced: tomlForced(found.trees.get("tool.sqlfluff.rules")) };
}

/** A layer for file `name`, not read for `why`, with a note saying so. */
function unreadLayer(name: string, why: string, notes: LinterNote[]): ConfigLayer {
  notes.push({ linter: "sqlfluff", text: `${name} is not read: ${why}` });
  return { unread: `${name} is not read` };
}

/** File `name` with `text` that is not read for `why`: not read, or nothing when it may hold no SQLFluff config at all. */
function failedLayer(name: string, text: string, why: string, notes: LinterNote[]): ConfigLayer | undefined {
  // A shared file that may hold no SQLFluff config at all is not worth a note.
  if (name !== ".sqlfluff" && !text.includes("sqlfluff")) return undefined;
  return unreadLayer(name, `${why}.`, notes);
}

/** Config file `name` at the root as a layer; undefined when it is absent or holds no SQLFluff config. */
function readLayer(root: string, name: string, notes: LinterNote[]): ConfigLayer | undefined {
  const read = readRootFile(root, name);
  if (!read.present) return undefined;
  const text = read.text;
  if (text === undefined) return unreadLayer(name, NOT_A_FILE, notes);
  const layer = readFormat(() => (name === PYPROJECT ? tomlLayer(text) : iniLayer(text, name)));
  return layer.ok ? layer.value : failedLayer(name, text, layer.why, notes);
}

/** What a setting comes to: its value in the last layer that sets it, unknown where a layer after that is not read. */
function resolve<T>(layers: readonly ConfigLayer[], valueIn: (layer: Exclude<ConfigLayer, { unread: string }>) => T | undefined): Resolved<T> {
  for (const layer of [...layers].reverse()) {
    if ("unread" in layer) return { unknown: layer.unread };
    const value = valueIn(layer);
    if (value !== undefined) return value;
  }
  return undefined;
}

/** Characters `chars` as a regular expression matching them literally, each escaped by its code point. */
const literal = (chars: readonly string[]): string => chars.map(char => `\\u{${char.codePointAt(0)!.toString(16)}}`).join("");

/**
 * The literal runs of an `fnmatch` class body (after any `!`), a range between each two, as `fnmatch.translate` splits
 * them: a `-` is a range only after the body's first character and past the character after the range before it, and
 * a range running backwards is dropped with its ends.
 */
function classChunks(body: readonly string[]): string[][] {
  const chunks: string[][] = [];
  let from = 0;
  for (let hyphen = body.indexOf("-", 1); hyphen !== -1; hyphen = body.indexOf("-", hyphen + 3)) {
    chunks.push(body.slice(from, hyphen));
    from = hyphen + 1;
  }
  const last = body.slice(from);
  if (last.length > 0) chunks.push(last);
  else chunks.at(-1)!.push("-");
  for (let index = chunks.length - 1; index > 0; index -= 1) {
    const before = chunks[index - 1]!;
    const after = chunks[index]!;
    if (before.at(-1)! > after[0]!) chunks.splice(index - 1, 2, [...before.slice(0, -1), ...after.slice(1)]);
  }
  return chunks;
}

/** `fnmatch`'s `[...]` class starting at `start`: its end and its regular expression, or undefined when it is not closed. */
function globClass(chars: readonly string[], start: number): { end: number; source: string } | undefined {
  let end = start + 1;
  if (chars[end] === "!") end += 1;
  if (chars[end] === "]") end += 1;
  end = chars.indexOf("]", end);
  if (end === -1) return undefined;
  const negated = chars[start + 1] === "!";
  const ranges = classChunks(chars.slice(negated ? start + 2 : start + 1, end)).map(literal).join("-");
  // A class left empty is `[]`, matching nothing, or `[^]`, matching anything: fnmatch's `(?!)` and `.`.
  return { end, source: `[${negated ? "^" : ""}${ranges}]` };
}

/** `glob` as `fnmatch.fnmatchcase` matches it: `*`, `?` and `[...]`, everything else literal. */
function globPattern(glob: string): RegExp {
  const chars = Array.from(glob);
  let source = "";
  for (let index = 0; index < chars.length; index += 1) {
    const char = chars[index]!;
    const found = char === "[" ? globClass(chars, index) : undefined;
    if (found !== undefined) {
      source += found.source;
      index = found.end;
    } else {
      source += char === "*" ? ".*" : char === "?" ? "." : literal([char]);
    }
  }
  return new RegExp(`^${source}$`, "su");
}

/** The first of `selection`'s references that selects a rule with references `names`. */
function selecting(selection: Selection, names: readonly string[]): string | undefined {
  return selection.references.find(reference => {
    const pattern = globPattern(reference);
    return names.some(name => pattern.test(name));
  });
}

/** Whether `resolved` is not known: a config Rerail does not read may set it. */
const isUnknown = <T>(resolved: Resolved<T>): resolved is { readonly unknown: string } => typeof resolved === "object" && resolved !== null && "unknown" in resolved;

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
  if (isUnknown(selection.rules)) return { on: "unknown", because: selection.rules.unknown };
  const selected = selectedBy(selection.rules, names, selection.labels);
  if (!selected.on) return selected;
  // A config not read may exclude what the rules select; it cannot select what they do not.
  if (isUnknown(selection.exclude)) return { on: "unknown", because: selection.exclude.unknown };
  return forcedBy(selection, names, selected);
}

export const sqlfluffReader: LinterReader = {
  linter: "sqlfluff",
  kinds: ["sqlfluff"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const layers = SQLFLUFF_FILES.flatMap(name => readLayer(root, name, notes) ?? []);
    if (layers.length === 0) return undefined;
    const selection: SqlfluffSelection = {
      rules: resolve(layers, layer => layer.keys.get("rules")),
      exclude: resolve(layers, layer => layer.keys.get("exclude_rules")),
      forced: new Map(FORCE_ENABLED.map(rule => [rule, resolve(layers, layer => layer.forced.get(rule))])),
      labels: layers.flatMap(layer => ("label" in layer ? [layer.label] : [])).join(" and "),
    };
    return {
      verdict(port): PortVerdict | undefined {
        const names = port.suppress.sqlfluff;
        return names.length === 0 ? undefined : { decision: decide(selection, names) };
      },
    };
  },
};
