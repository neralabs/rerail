/**
 * ESLint's config at a repository root, read as data into the rule entries ESLint applies: the flat config
 * (`eslint.config.*`, parsed and never run: `eslint-flat-config.ts`), else `.eslintrc.json` or `.eslintrc` (JSON), else
 * `package.json` `eslintConfig`. An eslintrc's `extends` it recognises (`eslint:recommended`,
 * `plugin:@typescript-eslint/recommended`, ...: `eslint-presets.ts`) contribute their entries before its own `rules`, its
 * `overrides` after them. Config code is never run or imported; `.eslintrc.js` and YAML configs are reported as not read.
 */
import { z } from "zod";
import { filesScope, readEslintFlatConfig, type FlatConfigEntry, type FlatConfigRead, type FlatConfigScope } from "./eslint-flat-config.js";
import { ESLINTRC_PRESETS, type EslintPresetEntry } from "./generated/eslint-presets.generated.js";
import { configFile, jsoncFile, jsonObject, severityOf, type ConfigFile, type SeverityReading } from "./jsonc-lint-config.js";
import { readFormat } from "./formats/lint-config-formats.js";
import { NOT_A_FILE, readRootFile, type LinterNote } from "./linter-config.js";

/** ESLint's flat config files: code, never run. */
const ESLINT_FLAT_CONFIGS = ["eslint.config.js", "eslint.config.mjs", "eslint.config.cjs", "eslint.config.ts", "eslint.config.mts", "eslint.config.cts"];
/** ESLint's legacy config files in the order ESLint looks for them; the first present is the one it uses. */
const ESLINTRC_FILES = [".eslintrc.js", ".eslintrc.cjs", ".eslintrc.yaml", ".eslintrc.yml", ".eslintrc.json", ".eslintrc"];
const ESLINTRC_CODE = new Set([".eslintrc.js", ".eslintrc.cjs"]);
export const NEVER_RUN = "Rerail never runs your config.";

/** A rule entry limited to some files (a flat config object's, an `.eslintrc` override's), after the last entry for its rule that every file gets. */
export interface ScopedRule {
  readonly key: string;
  readonly value: unknown;
  /** The folders it covers exactly, if it covers whole folders; `files` names them for the person. */
  readonly folders: readonly string[] | undefined;
  readonly files: string;
  /** What comes after it that Rerail does not read and that may replace it. */
  readonly shadowedBy?: string;
  /** The recognised shared config it comes from, as the config names it. */
  readonly preset?: string;
}

/** A flat config entry Rerail cannot read: why, and what after it Rerail does not read either and may replace it. */
export interface UnreadEntry {
  readonly why: string;
  readonly shadowedBy?: string;
}

/** The rules of an ESLint config, as ESLint applies its entries. */
export interface EslintRules {
  /** How the file is named to the person: `.eslintrc.json`, `package.json eslintConfig`. */
  readonly file: string;
  /** What every file gets, by rule key. */
  readonly rules: Readonly<Record<string, unknown>>;
  /** Rule keys of `rules` whose entry comes from a recognised shared config, with the config as the file names it. */
  readonly presets: ReadonlyMap<string, string>;
  /** Entries limited to some files, after the last entry for their rule that every file gets. */
  readonly scoped: readonly ScopedRule[];
  /** Rule keys whose value cannot be read without running the config: why, and what may replace it. */
  readonly unresolved: ReadonlyMap<string, UnreadEntry>;
  /** Rule keys of `rules` that something after them Rerail does not read may replace, with what. */
  readonly shadowed: ReadonlyMap<string, string>;
  /**
   * Rule keys of `rules` whose severity is known but whose options something Rerail does not read may have set (a
   * severity alone after it keeps them), with what.
   */
  readonly optionsUnknown: ReadonlyMap<string, string>;
  /** The first thing the config takes and Rerail does not read, which may set any rule; none when it is read whole. */
  readonly unread?: string;
  /**
   * Why the file is not statically readable, if it is not: Rerail cannot tell what it turns off, so every rule it
   * ports from this linter starts off, with no options.
   */
  readonly unreadable?: string;
  /** How its severities read: an eslintrc's (`"OFF"` is `"off"`, as ESLint's eslintrc loader lower-cases them) or a flat config's. */
  readonly severities: Exclude<SeverityReading, "oxlint">;
}

/** A severity with no options: a severity, or a list of just one, which ESLint merges as a severity alone. */
export const severityAlone = (value: unknown): boolean => !Array.isArray(value) || value.length === 1;

/** A severity with no options after it, as a later config object sets one: ESLint keeps the earlier options. */
function withEarlierOptions(earlier: unknown, later: unknown): unknown {
  if (!severityAlone(later) || !Array.isArray(earlier) || earlier.length <= 1) return later;
  return [Array.isArray(later) ? later[0] : later, ...earlier.slice(1)];
}

/** A config applied so far: `rules` and `scoped` as they stand, and what they may not say for sure. */
interface RulesState {
  readonly severities: Exclude<SeverityReading, "oxlint">;
  readonly rules: Record<string, unknown>;
  readonly presets: Map<string, string>;
  readonly unresolved: Map<string, UnreadEntry>;
  readonly shadowed: Map<string, string>;
  readonly optionsUnknown: Map<string, string>;
  /** Per rule key, the unread entries for some files that no later entry has replaced, with how the config names the files. */
  readonly scopedUnresolved: Map<string, { readonly files: string; readonly entry: UnreadEntry }[]>;
  scoped: ScopedRule[];
}

const unreadEntryOf = (why: string, by: string | undefined): UnreadEntry => (by === undefined ? { why } : { why, shadowedBy: by });

/** Whether `value` turns the rule off, as the config reads severities. */
const isOff = (state: RulesState, value: unknown): boolean => severityOf(value, state.severities)?.off === true;

/** A read entry limited to some files, as kept apart. */
function scopedRuleOf({ key, shadowedBy, preset }: FlatConfigEntry, value: unknown, scope: Exclude<FlatConfigScope, { kind: "all" }>): ScopedRule {
  return {
    key,
    value,
    folders: scope.kind === "folders" ? scope.folders : undefined,
    files: scope.files,
    ...(shadowedBy === undefined ? {} : { shadowedBy }),
    ...(preset === undefined ? {} : { preset }),
  };
}

/** An entry limited to some files: kept apart, and a value in full replaces the unread entries for the same files. */
function applyScopedEntry(state: RulesState, entry: FlatConfigEntry, scope: Exclude<FlatConfigScope, { kind: "all" }>): void {
  const { key, value } = entry;
  const unreadBefore = state.scopedUnresolved.get(key) ?? [];
  if (!value.ok) {
    state.scopedUnresolved.set(key, [...unreadBefore, { files: scope.files, entry: unreadEntryOf(`${value.why} (for ${scope.files})`, entry.shadowedBy) }]);
    return;
  }
  state.scoped.push(scopedRuleOf(entry, value.value, scope));
  if (!severityAlone(value.value) || isOff(state, value.value)) {
    state.scopedUnresolved.set(key, unreadBefore.filter(unread => unread.files !== scope.files));
  }
}

/**
 * What an entry every file gets does to the earlier entries for some files. A severity alone ("off" included, as
 * ESLint's rulesSchema.merge does) keeps the earlier options, for every file and for the files an earlier entry limited
 * to some files set them for; a severity with options replaces them.
 */
function replaceScopedEntries(state: RulesState, key: string, value: FlatConfigEntry["value"]): void {
  if (value.ok && severityAlone(value.value)) {
    state.scoped = state.scoped.flatMap(entry => (entry.key !== key ? [entry]
      : severityAlone(entry.value) ? []
      : [{ ...entry, value: withEarlierOptions(entry.value, value.value) }]));
  } else {
    state.scoped = state.scoped.filter(entry => entry.key !== key);
    state.scopedUnresolved.delete(key);
  }
}

/** Sets `map`'s entry for `key` to `value`, or deletes it for undefined. */
function setOrDelete<V>(map: Map<string, V>, key: string, value: V | undefined): void {
  if (value === undefined) map.delete(key);
  else map.set(key, value);
}

/** A value every file gets replaces what `rules` held for the key, and says what may still be uncertain about it. */
function setEveryFileRule(state: RulesState, entry: FlatConfigEntry, value: unknown, earlier: UnreadEntry | undefined): void {
  const { key } = entry;
  state.unresolved.delete(key);
  // What an unread element after the earlier entry may have set: its severity is now this entry's, its options not.
  const uncertain = severityAlone(value) ? state.shadowed.get(key) ?? state.optionsUnknown.get(key) ?? earlier?.why : undefined;
  Object.defineProperty(state.rules, key, { value: withEarlierOptions(state.rules[key], value), enumerable: true, writable: true, configurable: true });
  setOrDelete(state.shadowed, key, entry.shadowedBy);
  setOrDelete(state.optionsUnknown, key, uncertain);
  setOrDelete(state.presets, key, entry.preset);
}

/** An entry every file gets whose value Rerail cannot read: it stands for the rule until a later entry replaces it. */
function setUnresolvedRule(state: RulesState, { key, value, shadowedBy }: FlatConfigEntry & { value: { ok: false } }): void {
  state.unresolved.set(key, unreadEntryOf(value.why, shadowedBy));
  delete state.rules[key];
  for (const map of [state.shadowed, state.optionsUnknown, state.presets]) map.delete(key);
}

/** An entry every file gets replaces the earlier ones, keeping their options when it sets only a severity. */
function applyEveryFileEntry(state: RulesState, entry: FlatConfigEntry): void {
  const { key, value, shadowedBy } = entry;
  replaceScopedEntries(state, key, value);
  const earlier = state.unresolved.get(key);
  if (!value.ok) setUnresolvedRule(state, { ...entry, value });
  // On, with options Rerail cannot tell.
  else if (earlier !== undefined && severityAlone(value.value) && !isOff(state, value.value)) state.unresolved.set(key, unreadEntryOf(earlier.why, shadowedBy ?? earlier.shadowedBy));
  else setEveryFileRule(state, entry, value.value, earlier);
}

/** An unread entry for some files that no later entry replaced stands for its rule. */
function keepScopedUnresolved(state: RulesState): void {
  for (const [key, entries] of state.scopedUnresolved) {
    if (!state.unresolved.has(key) && entries[0] !== undefined) state.unresolved.set(key, entries[0].entry);
  }
}

/**
 * A config's entries as ESLint applies them, in order: a later entry every file gets replaces the earlier ones (keeping
 * their options when it sets only a severity), and the entries limited to some files that come after it are kept
 * apart. An entry Rerail cannot read stands for its rule until a later entry replaces it: one for every file, or one
 * for the same files that sets the rule in full. What a later unread element may replace stays so through a severity
 * alone, which keeps whatever options it set.
 */
function applyEntries(file: string, read: Pick<FlatConfigRead, "entries" | "unread">, severities: RulesState["severities"]): EslintRules {
  const state: RulesState = {
    severities, rules: {}, presets: new Map(), unresolved: new Map(), shadowed: new Map(), optionsUnknown: new Map(), scopedUnresolved: new Map(), scoped: [],
  };
  for (const entry of read.entries) {
    if (entry.scope.kind === "all") applyEveryFileEntry(state, entry);
    else applyScopedEntry(state, entry, entry.scope);
  }
  const { rules, presets, scoped, unresolved, shadowed, optionsUnknown } = state;
  keepScopedUnresolved(state);
  const unread = read.unread[0];
  return { file, rules, presets, scoped, unresolved, shadowed, optionsUnknown, severities, ...(unread === undefined ? {} : { unread }) };
}

type Entry = { -readonly [K in keyof FlatConfigEntry]: FlatConfigEntry[K] };

/** An eslintrc's entries as they are gathered, in the order ESLint applies them, and what is not read among them. */
class EslintrcEntries {
  readonly entries: Entry[] = [];
  readonly unread: string[] = [];

  /** The rules of `rules` for `scope`, from `preset` where they come from one. */
  add(rules: Readonly<Record<string, unknown>>, scope: FlatConfigScope, preset?: string): void {
    for (const [key, value] of Object.entries(rules)) this.entries.push({ key, scope, value: { ok: true, value }, ...(preset === undefined ? {} : { preset }) });
  }

  /** Something not read that may set any rule: every entry before it may be replaced by it. */
  shadow(by: string): void {
    this.unread.push(by);
    for (const entry of this.entries) entry.shadowedBy ??= by;
  }
}

/** An eslintrc `files` value (and `excludedFiles`) as a scope: whole folders where every pattern is one and none is excluded. */
function eslintrcScope(files: string | readonly unknown[], excluded: boolean): FlatConfigScope {
  const text = JSON.stringify(files);
  return excluded ? { kind: "some", files: text } : filesScope(typeof files === "string" ? [files] : files, text);
}

/** A recognised preset's entries, each for its own files, from the eslintrc that extends it. */
function addPreset(gathered: EslintrcEntries, name: string, entries: readonly EslintPresetEntry[]): void {
  for (const entry of entries) {
    gathered.add(entry.rules, entry.files === undefined ? { kind: "all" } : eslintrcScope(entry.files, entry.ignores !== undefined), name);
  }
}

/** An eslintrc's `extends`: recognised presets in place, any other not read; the names not read. */
function addExtends(gathered: EslintrcEntries, file: string, extended: unknown): string[] {
  if (extended === undefined) return [];
  const names = typeof extended === "string" ? [extended] : extended;
  if (!Array.isArray(names) || !names.every(name => typeof name === "string")) {
    gathered.shadow(`${file} extends`);
    return [`${file} extends`];
  }
  return names.flatMap(name => {
    const preset = ESLINTRC_PRESETS.get(name);
    if (preset !== undefined) addPreset(gathered, name, preset);
    else gathered.shadow(`${file} extends ${JSON.stringify(name)}`);
    return preset === undefined ? [name] : [];
  });
}

const eslintrcOverride = z.looseObject({
  files: z.union([z.string(), z.array(z.string())]),
  rules: jsonObject.optional(),
});

/** One override of an eslintrc: its rules for its files, what in it is not read shadowing what comes before. */
function addOverride(gathered: EslintrcEntries, file: string, index: number, value: unknown, notes: LinterNote[]): void {
  const override = eslintrcOverride.safeParse(value);
  if (!override.success) {
    notes.push({ linter: "eslint", text: `${file}: overrides[${index}] is not read: it has no files or its rules are not an object.` });
    gathered.shadow(`${file} overrides[${index}]`);
    return;
  }
  const unread = ["extends", "overrides"].filter(key => Object.hasOwn(override.data, key));
  if (unread.length > 0) notes.push({ linter: "eslint", text: `${file}: overrides[${index}]: only its own rules are read, not ${unread.join(" or ")}.` });
  // An override's extends applies before its rules, its nested overrides after them.
  if (Object.hasOwn(override.data, "extends")) gathered.shadow(`${file} overrides[${index}].extends`);
  gathered.add(override.data.rules ?? {}, eslintrcScope(override.data.files, Object.hasOwn(override.data, "excludedFiles")));
  if (Object.hasOwn(override.data, "overrides")) gathered.shadow(`${file} overrides[${index}].overrides`);
}

/**
 * An `.eslintrc`'s `overrides`, in order: ESLint applies each after the config's own rules to the files its `files`
 * match, less its `excludedFiles`. What Rerail cannot read in them may replace any rule entry before it.
 */
function addOverrides(gathered: EslintrcEntries, file: string, overrides: unknown, notes: LinterNote[]): void {
  if (overrides === undefined) return;
  if (!Array.isArray(overrides)) {
    notes.push({ linter: "eslint", text: `${file}: its overrides are not read: they are not a list.` });
    gathered.shadow(`${file} overrides`);
    return;
  }
  for (const [index, value] of overrides.entries()) addOverride(gathered, file, index, value, notes);
}

/**
 * An eslintrc config `config` (an `.eslintrc` file, `package.json` `eslintConfig`) as ESLint applies it: its `extends`
 * (recognised presets in place), its own `rules`, its `overrides`. Undefined (with a note) when `rules` is not an object.
 */
function eslintrcRules(file: string, config: ConfigFile, notes: LinterNote[]): EslintRules | undefined {
  const rules = jsonObject.safeParse(config.rules ?? {});
  if (!rules.success) {
    notes.push({ linter: "eslint", text: `${file} is not read: its rules are not an object.` });
    return undefined;
  }
  const gathered = new EslintrcEntries();
  const unreadExtends = addExtends(gathered, file, config.extends);
  const skipped = [...unreadExtends.length > 0 ? ["extends"] : [], ...Object.hasOwn(config, "plugins") ? ["plugins"] : []];
  if (skipped.length > 0) notes.push({ linter: "eslint", text: `${file}: only its own rules are read, not ${skipped.join(" or ")}.` });
  gathered.add(rules.data, { kind: "all" });
  addOverrides(gathered, file, config.overrides, notes);
  return applyEntries(file, gathered, "eslintrc");
}

/** A flat config Rerail cannot read statically: it cannot tell what it turns off, so every ESLint port starts off. */
function unreadableFlat(file: string, why: string, notes: LinterNote[]): EslintRules {
  notes.push({ linter: "eslint", text: `${file} is not statically readable: ${why}. ${NEVER_RUN} The rules Rerail ports from ESLint start off, since Rerail cannot tell which it turns off.` });
  return { file, rules: {}, presets: new Map(), scoped: [], unresolved: new Map(), shadowed: new Map(), optionsUnknown: new Map(), severities: "eslint", unreadable: why };
}

/** A config file present at the root: its name, and its text where it is a regular file Rerail reads. */
interface PresentFile {
  readonly name: string;
  readonly text: string | undefined;
}

/** The files of `names` present at `root`, in order. */
function presentFiles(root: string, names: readonly string[]): PresentFile[] {
  return names.flatMap(name => {
    const read = readRootFile(root, name);
    return read.present ? [{ name, text: read.text }] : [];
  });
}

/** The flat config file ESLint uses, parsed and never run. */
function flatRules({ name, text }: PresentFile, notes: LinterNote[]): EslintRules {
  if (text === undefined) return unreadableFlat(name, NOT_A_FILE.replace(/\.$/u, ""), notes);
  const config = readFormat(() => readEslintFlatConfig(text, name));
  if (!config.ok) return unreadableFlat(name, config.why, notes);
  for (const note of config.value.notes) notes.push({ linter: "eslint", text: `${name}: ${note}` });
  return applyEntries(name, config.value, "eslint");
}

/** The eslintrc file `used`: JSON read, code and YAML not. */
function eslintrcFile(root: string, used: string, notes: LinterNote[]): EslintRules | undefined {
  if (ESLINTRC_CODE.has(used)) {
    notes.push({ linter: "eslint", text: `${used} is not read: ${NEVER_RUN}` });
    return undefined;
  }
  if (used.endsWith(".yaml") || used.endsWith(".yml")) {
    notes.push({ linter: "eslint", text: `${used} is not read: Rerail reads JSON lint configs only.` });
    return undefined;
  }
  const config = jsoncFile(root, used, "eslint", notes);
  return config === undefined ? undefined : eslintrcRules(used, config, notes);
}

/** `package.json`'s `eslintConfig`, which ESLint reads where there is no config file. */
function manifestConfig(root: string, notes: LinterNote[]): EslintRules | undefined {
  const manifest = readRootFile(root, "package.json");
  if (!manifest.present || manifest.text === undefined) return undefined;
  let parsed: unknown;
  try {
    parsed = JSON.parse(manifest.text);
  } catch {
    // An unreadable manifest is not an ESLint config; the matcher skips it the same way.
    return undefined;
  }
  const eslintConfig = z.looseObject({ eslintConfig: z.unknown().optional() }).safeParse(parsed).data?.eslintConfig;
  if (eslintConfig === undefined) return undefined;
  const config = configFile.safeParse(eslintConfig);
  if (config.success) return eslintrcRules("package.json eslintConfig", config.data, notes);
  notes.push({ linter: "eslint", text: "package.json eslintConfig is not read: it is not an object." });
  return undefined;
}

/** Notes the config files ESLint does not use, the one it uses being `used`. */
function noteIgnored(ignored: readonly string[], used: string, notes: LinterNote[]): void {
  for (const name of ignored) notes.push({ linter: "eslint", text: `${name} is not read: ESLint uses ${used} instead.` });
}

/** The ESLint config ESLint uses at `root`, read as data; undefined when there is none Rerail reads. */
export function readEslintRules(root: string, notes: LinterNote[]): EslintRules | undefined {
  const [used, ...ignored] = presentFiles(root, ESLINT_FLAT_CONFIGS);
  const legacy = ESLINTRC_FILES.filter(name => readRootFile(root, name).present);
  if (used !== undefined) {
    // ESLint uses the first flat config it finds wherever there is one; the files it then ignores are named after
    // what Rerail made of that one.
    const rules = flatRules(used, notes);
    noteIgnored([...ignored.map(file => file.name), ...legacy], used.name, notes);
    return rules;
  }
  if (legacy.length === 0) return manifestConfig(root, notes);
  noteIgnored(legacy.slice(1), legacy[0]!, notes);
  return eslintrcFile(root, legacy[0]!, notes);
}
