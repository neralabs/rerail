/**
 * Biome's config at a repository root, read as data as Biome 2.5.15 reads it (never run), for the ports that honour
 * `biome-ignore` comments: their `lint/<group>/<rule>` names.
 *
 * Biome takes the first of `biome.json`, `biome.jsonc`, `.biome.json`, `.biome.jsonc` in a folder. Its `linter`
 * section decides which rules run: `enabled`, `rules` (a preset, `recommended`, groups set to a severity or listing
 * rules) and `domains`; `extends` merges other config files under it and `overrides` merges more `linter` sections
 * over it for the files they include. Which rules a preset or a domain holds comes from Biome's own rule metadata
 * (`biome-rules.generated.ts`). What the config takes and Rerail cannot read (a package in `extends`, a file outside
 * the repository, a root config above the repository) leaves the rules it may set at the pack's values.
 */
import { posix } from "node:path";
import { BIOME_DOMAINS, BIOME_RULES, type BiomeRuleDeclaration } from "./generated/biome-rules.generated.js";
import { folderOfGlob } from "./eslint-flat-config.js";
import { LintConfigFormatError, parseJsonc, readFormat } from "./formats/lint-config-formats.js";
import {
  NOT_A_FILE,
  readRootFile,
  type FileRead,
  type FolderDecision,
  type LinterConfig,
  type LinterNote,
  type LinterReader,
  type Port,
  type PortDecision,
  type PortVerdict,
} from "./linter-config.js";

/** Biome's config file names in the order it looks for them in a folder (`ConfigName` in crates/biome_fs/src/fs.rs l.24-25). */
const BIOME_FILES = ["biome.json", "biome.jsonc", ".biome.json", ".biome.jsonc"];
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

/** An override's `linter` section, for the whole folders its `includes` name. */
interface FolderOverride {
  readonly folders: readonly string[];
  readonly layer: LinterLayer;
}

/** One config file as read: whether it is a root config, what it extends, its `linter` and its overrides. */
interface ConfigFile {
  readonly root: boolean;
  readonly extends: readonly string[];
  readonly linter: LinterLayer | undefined;
  readonly overrides: readonly FolderOverride[];
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

/** An override's `includes` as whole folders; undefined when one is not `<folder>/**`. */
function overrideFolders(includes: unknown, where: string): string[] | undefined {
  if (includes === undefined) return undefined;
  if (!Array.isArray(includes) || !includes.every(glob => typeof glob === "string")) fail(where, "is not a list of globs");
  const folders = includes.map(folderOfGlob);
  return folders.length > 0 && folders.every(folder => folder !== undefined) ? folders as string[] : undefined;
}

/** `overrides[index]` of `file` with a `linter` section, for whole folders; a note for one that is for other files. */
function folderOverride(value: unknown, file: string, index: number, notes: LinterNote[]): FolderOverride[] {
  const path = `overrides[${index}]`;
  const override = objectAt(value, path);
  if (override.linter === undefined) return [];
  const layer = linterLayer(override.linter, file, `${path}.linter`);
  const folders = overrideFolders(override.includes, `${path}.includes`);
  if (folders !== undefined) return [{ folders, layer }];
  notes.push({ linter: "biome", text: `${file}: ${path} is not read: its includes ${JSON.stringify(override.includes ?? [])} are not whole folders (<folder>/**).` });
  return [];
}

/** `extends`: a list of config files, or `"//"` (the root config of a nested one), as `Extends` reads it (crates/biome_configuration/src/extends.rs). */
function extendsOf(value: unknown): { list: string[]; root: boolean } {
  if (value === undefined) return { list: [], root: false };
  if (value === "//") return { list: [], root: true };
  if (!Array.isArray(value) || !value.every(entry => typeof entry === "string")) fail("extends", "is not \"//\" or a list of paths");
  return { list: value, root: false };
}

/** Config file `file`'s object `config`; `root` as `Configuration::is_root` (crates/biome_configuration/src/lib.rs l.361-366). */
function configFile(config: Record<string, unknown>, file: string, notes: LinterNote[]): ConfigFile {
  const extended = extendsOf(config.extends);
  const root = config.root === undefined ? !extended.root : flag(config.root, "root");
  const overrides = config.overrides ?? [];
  if (!Array.isArray(overrides)) fail("overrides", "is not a list");
  if (isObject(config.linter) && config.linter.includes !== undefined) {
    notes.push({ linter: "biome", text: `${file}: linter.includes is not read; the rules Rerail sets apply to every file.` });
  }
  return {
    root,
    extends: extended.list,
    linter: config.linter === undefined ? undefined : linterLayer(config.linter, file, "linter"),
    overrides: overrides.flatMap((override: unknown, index) => folderOverride(override, file, index, notes)),
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
  const parsed = readFormat(() => configFile(parseConfig(name, text), name, notes));
  if (parsed.ok) return parsed.value;
  notes.push({ linter: "biome", text: `${name} is not read: ${parsed.why}.` });
  return undefined;
}

/**
 * A file in `extends`, relative to the root; undefined for a package or a file outside the repository. A relative
 * entry (`is_relative_specifier`, crates/biome_resolver/src/lib.rs l.865-867) is a path; any other is first tried as a
 * path from the working directory (`with_assume_relative`), taken here as the root, then as a package.
 */
function extendedPath(root: string, entry: string): string | undefined {
  const path = posix.normalize(entry);
  if (/^(?:\/|\.\.(?:\/|$))/u.test(path)) return undefined;
  return /^\.\.?(?:\/|$)/u.test(entry) || readRootFile(root, path).present ? path : undefined;
}

/** The `linter` of a config file that has none: Biome's defaults. */
const NO_LINTER: LinterLayer = { domains: new Map() };

/** What `extends` of `file` merges under it, in order: each config's `linter` (not its own `extends`, which Biome does not follow), or `UNREAD`. */
function extendedLayers(root: string, file: string, config: ConfigFile, notes: LinterNote[]): { layers: (LinterLayer | Unread)[]; overrides: FolderOverride[]; unread: string | undefined } {
  const layers: (LinterLayer | Unread)[] = [];
  const overrides: FolderOverride[] = [];
  let unread: string | undefined;
  for (const entry of config.extends) {
    const path = extendedPath(root, entry);
    if (path === undefined) notes.push({ linter: "biome", text: `${file}: extends ${JSON.stringify(entry)} is not read: it is a package or a file outside the repository; the rules it may set keep the pack's values.` });
    const extended = path === undefined ? undefined : configOf(path, readRootFile(root, path), notes);
    if (extended === undefined) unread = `${file} extends ${JSON.stringify(entry)}, which Rerail does not read`;
    layers.push(extended === undefined ? UNREAD : extended.linter ?? NO_LINTER);
    overrides.push(...extended?.overrides ?? []);
  }
  return { layers, overrides, unread };
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

/** What reading a config for a verdict needs: the file Biome takes, and why some of it is not known. */
interface Context {
  readonly file: string;
  readonly unread: string;
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
 * the domain's packages (`ManifestVisitor`, file_handlers/mod.rs l.2002-2063): Rerail does not tell, so it may.
 */
function manifestEffects(folded: Folded, rule: BiomeRule, context: Context): Effect[] {
  if (rule.nursery || recommendedEnabled(folded) === false) return [];
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

/** What the merged `layers` decide for `rule`. */
function ruleDecision(layers: readonly (LinterLayer | Unread)[], rule: BiomeRule, context: Context): PortDecision {
  const folded = layers.reduce((state, layer) => foldLayer(state, layer, rule), EMPTY);
  if (isFact(folded.enabled) && !folded.enabled.value) return { on: false, explicit: false, because: folded.enabled.by };
  const named = explicitlyOn(folded, rule);
  const domainRule = rule.nursery ? [] : rule.declarations.flatMap(declaration => declarationDomainEffects(folded, declaration, named, context));
  return decide([...ruleEffects(folded, rule, context), ...domainRule, ...manifestEffects(folded, rule, context)], context);
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
function nameDecision(layers: readonly (LinterLayer | Unread)[], name: string, context: Context): PortDecision {
  const rule = biomeRule(name);
  return rule === undefined ? { on: "unknown", because: `${name} is not a Biome 2.5.15 lint rule` } : ruleDecision(layers, rule, context);
}

/** Biome's config read at the root, ready to give verdicts. */
interface BiomeView {
  readonly context: Context;
  readonly layers: readonly (LinterLayer | Unread)[];
  readonly overrides: readonly FolderOverride[];
}

/** Per whole folder an override is for, the port's state there when it differs from every file's: the overrides for it or a folder holding it apply, in order. */
function folderDecisions(view: BiomeView, names: readonly string[], whole: PortDecision): FolderDecision[] {
  const folders = [...new Set(view.overrides.flatMap(override => override.folders))].sort();
  return folders.flatMap(folder => {
    const applying = view.overrides.filter(override => override.folders.some(holder => folder === holder || folder.startsWith(`${holder}/`)));
    const layers = [...view.layers, ...applying.map(override => override.layer)];
    const decision = portDecision(names.map(name => nameDecision(layers, name, view.context)));
    if (decision.on === "unknown" || decision.on === whole.on) return [];
    return [{ folder, on: decision.on, because: decision.because }];
  });
}

/** The verdicts of Biome's config as `view` holds it. */
function biomeVerdicts(view: BiomeView): LinterConfig {
  return {
    verdict(port: Port): PortVerdict | undefined {
      const names = port.suppress.biome;
      if (names.length === 0) return undefined;
      const decision = portDecision(names.map(name => nameDecision(view.layers, name, view.context)));
      const folders = folderDecisions(view, names, decision);
      return { decision, ...folders.length > 0 ? { folders } : {} };
    },
  };
}

/** Biome's config `file` at `root`, as `read` holds it, and what it extends, read as Biome merges them. */
function readBiomeConfig(root: string, file: string, read: FileRead, notes: LinterNote[]): LinterConfig | undefined {
  const config = configOf(file, read, notes);
  if (config === undefined) return undefined;
  if (!config.root) {
    notes.push({ linter: "biome", text: `${file} is not read: it is not a root config ("root": false, or "extends": "//"), so Biome takes a config outside the repository.` });
    const because = `${file} is not Biome's root config`;
    return { verdict: port => (port.suppress.biome.length === 0 ? undefined : { decision: { on: "unknown", because } }) };
  }
  const extended = extendedLayers(root, file, config, notes);
  return biomeVerdicts({
    context: { file, unread: extended.unread ?? `${file} takes a config Rerail does not read` },
    layers: [...extended.layers, config.linter ?? NO_LINTER],
    overrides: [...extended.overrides, ...config.overrides],
  });
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
