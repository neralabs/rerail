/**
 * What a repository's ESLint config (`eslint-rules.ts`) says about each port of ESLint rules: the verdict set-up
 * settles with the other linters'. A rule an entry turns on or off is decided explicitly (with its options mapped onto
 * the port's `config` where it declares them), one a recognised shared config's entry turns on or off implicitly. A
 * rule the config never sets is off, as ESLint leaves it, when Rerail reads the whole config, and unknown (the pack's
 * value stands) when something it takes and Rerail does not read may set it. An entry for a whole folder's files turns
 * the rule on or off in that folder; one for files narrower than a folder turns it on everywhere (it runs there) and
 * leaves an off with a note. A flat config Rerail cannot read statically turns every port off.
 */
import type { CodeRuleDslValue } from "../../code-rule/code-rule.js";
import { stableJsonText } from "../../stable-json.js";
import type { RuleCatalog } from "../repository-model.js";
import { configureBundledCodeRule } from "../rule-config.js";
import { readEslintRules, type EslintRules, type ScopedRule, type UnreadEntry } from "./eslint-rules.js";
import { entryText, jsonObject, severityOf, type Severity } from "./jsonc-lint-config.js";
import { combineDecisions } from "./linter-decisions.js";
import type { Declared, FolderDecision, LinterConfig, LinterNote, LinterReader, MappedOptions, Port, PortDecision, PortVerdict } from "./linter-config.js";

const isOptionValue = (value: unknown): value is CodeRuleDslValue =>
  value === null || ["string", "number", "boolean"].includes(typeof value)
  || (Array.isArray(value) && value.every(item => item === null || ["string", "number", "boolean"].includes(typeof item)));

/** Sets option `name` to `value` in `config`, or says why it cannot. */
function setOption(declared: Declared, config: Record<string, CodeRuleDslValue>, name: string, value: unknown): string | undefined {
  if (!Object.hasOwn(declared, name)) return `${name} is not an option the port declares`;
  if (!isOptionValue(value)) return `${name}: ${JSON.stringify(value)} is not a value Rerail maps`;
  if (Object.hasOwn(config, name)) return `${name} is set twice`;
  config[name] = value;
  return undefined;
}

/** One positional option set into `config`: an object's keys by themselves (a nested object's as `outer.inner`), any other value as `option`, `option2`, ... */
function setPositional(declared: Declared, config: Record<string, CodeRuleDslValue>, index: number, option: unknown): string | undefined {
  const object = jsonObject.safeParse(option);
  if (!object.success) return setOption(declared, config, index === 0 ? "option" : `option${index + 1}`, option);
  const problems = Object.entries(object.data).map(([key, value]) => {
    const nested = jsonObject.safeParse(value);
    if (!nested.success) return setOption(declared, config, key, value);
    return Object.entries(nested.data).map(([inner, innerValue]) => setOption(declared, config, `${key}.${inner}`, innerValue)).find(problem => problem !== undefined);
  });
  return problems.find(problem => problem !== undefined);
}

/**
 * ESLint's options for a rule as the port's `config`: the ports name the first positional option `option` and the
 * second `option2`, an option object's keys by themselves and a nested object's keys `outer.inner`. Every value must
 * land on an option the document declares; otherwise nothing is mapped.
 */
function eslintOptionsConfig(declared: Declared, options: readonly unknown[]): MappedOptions {
  const config: Record<string, CodeRuleDslValue> = {};
  for (const [index, option] of options.entries()) {
    const problem = setPositional(declared, config, index, option);
    if (problem !== undefined) return { ok: false, reason: problem };
  }
  return { ok: true, config };
}

/** One port read against one ESLint config: what it gathers as it reads the port's rule keys. */
interface PortRead {
  readonly ruleCatalog: RuleCatalog;
  readonly port: Port;
  readonly eslint: EslintRules;
  readonly decisions: PortDecision[];
  readonly folders: FolderDecision[];
  readonly notes: string[];
}

/** ESLint options as the port's `config`, only where the port accepts them as a stored rule setting would be. */
function mappedOptions(read: PortRead, options: readonly unknown[]): MappedOptions {
  const config = eslintOptionsConfig(read.port.declared, options);
  if (!config.ok) return config;
  const configured = configureBundledCodeRule(read.ruleCatalog, read.port.ruleId, config.config);
  return configured.ok ? config : { ok: false, reason: configured.reason };
}

const replacedBy = (by: string): string => `${by}, after it, is not read and may replace it`;
const severity = (read: PortRead, value: unknown): Severity | undefined => severityOf(value, read.eslint.severities);

/** The entry after these that turns the rule back on: it may cover any of the files one that turns it off is for. */
function turnedBackOnBy(read: PortRead, after: readonly ScopedRule[]): ScopedRule | undefined {
  return after.find(entry => severity(read, entry.value)?.off === false);
}

/** How an entry is named to the person: the entry itself, or the shared config it comes from. */
const entryBecause = (read: PortRead, key: string, value: unknown, preset: string | undefined): string =>
  (preset === undefined ? entryText(read.eslint.file, key, value) : `${read.eslint.file}: ${preset}`);

/** The repository sets the rule, to what Rerail cannot tell without running the config. */
function readUnresolvedKey(read: PortRead, key: string, entry: UnreadEntry): void {
  const because = `${read.eslint.file}: ${JSON.stringify(key)} is not read (${entry.why})`;
  // Unless something after it that Rerail cannot read either may replace it, in which case it is not used at all.
  if (entry.shadowedBy === undefined) {
    read.decisions.push({ on: false, explicit: true, because });
    return;
  }
  read.notes.push(`${read.port.ruleId}: ${because} and is not used: ${replacedBy(entry.shadowedBy)}.`);
  read.decisions.push({ on: "unknown", because });
}

/** An entry for some files that sets options other than these, which would change what ESLint reports there. */
function otherScopedOptions(read: PortRead, options: readonly unknown[], later: readonly ScopedRule[]): ScopedRule | undefined {
  return later.find(entry => {
    const scoped = entry.shadowedBy === undefined ? severity(read, entry.value) : undefined;
    return scoped !== undefined && !scoped.off && scoped.options.length > 0 && stableJsonText(scoped.options) !== stableJsonText(options);
  });
}

/** Notes why the options of `because` are not used: something unread before it may have set them, or an entry for some files sets others. */
function noteUnusedOptions(read: PortRead, key: string, because: string, unknown: string | undefined, other: ScopedRule | undefined): void {
  const { eslint, port: { ruleId }, notes } = read;
  if (unknown !== undefined) notes.push(`${ruleId}: the options of ${because} are not used: ${unknown}, before it, is not read and may have set them.`);
  if (other !== undefined) {
    notes.push(`${ruleId}: the options of ${because} are not used: ${entryText(eslint.file, key, other.value)} sets others for ${other.files}, and Rerail sets options for the whole repository only.`);
  }
}

/** The mapped options of an entry for every file that sets some, saying why when they cannot be used. */
function everyFileOptions(read: PortRead, key: string, found: Severity, because: string, later: readonly ScopedRule[]): MappedOptions | undefined {
  if (found.off || found.options.length === 0) return undefined;
  // Options for every file are the port's for the whole repository only when no entry for some files sets others.
  const other = otherScopedOptions(read, found.options, later);
  const unknown = read.eslint.optionsUnknown.get(key);
  noteUnusedOptions(read, key, because, unknown, other);
  return unknown === undefined && other === undefined ? mappedOptions(read, found.options) : undefined;
}

/** Why an entry is not used, if it is not: not a severity ESLint takes, or something unread after it may replace it. */
function unusedEntry(read: PortRead, because: string, found: Severity | undefined, shadowedBy: string | undefined): string | undefined {
  if (found === undefined) return `${read.port.ruleId}: ${because} is not a severity ESLint takes; not read.`;
  return shadowedBy === undefined ? undefined : `${read.port.ruleId}: ${because} is not used: ${replacedBy(shadowedBy)}.`;
}

/**
 * Whether an entry is not used: then it is noted (unless a shared config's, whose entries are not the person's to
 * fix) and leaves the rule unknown.
 */
function skipUnused(read: PortRead, unused: string | undefined, preset: string | undefined, because: string): boolean {
  if (unused === undefined) return false;
  if (preset === undefined) read.notes.push(unused);
  read.decisions.push({ on: "unknown", because });
  return true;
}

/** An entry for every file that turns the rule off, where a later entry for some files turns it back on. */
function readTurnedBackOn(read: PortRead, key: string, because: string, back: ScopedRule): void {
  // Back on for whole folders: off elsewhere, and on there (`readScopedEntry`).
  if (back.folders !== undefined) {
    read.decisions.push({ on: false, explicit: true, because });
    return;
  }
  read.notes.push(`${read.port.ruleId}: ${because} turns it off, but ${entryText(read.eslint.file, key, back.value)} turns it back on for ${back.files}; it stays on.`);
  read.decisions.push({ on: true, explicit: back.preset === undefined, because: `${entryBecause(read, key, back.value, back.preset)} for ${back.files}` });
}

/** What the config says about `key` for every file. */
function readEveryFileEntry(read: PortRead, key: string, later: readonly ScopedRule[]): void {
  const value = read.eslint.rules[key];
  const preset = read.eslint.presets.get(key);
  const because = entryBecause(read, key, value, preset);
  const found = severity(read, value);
  if (skipUnused(read, unusedEntry(read, entryText(read.eslint.file, key, value), found, read.eslint.shadowed.get(key)), preset, because)) return;
  const back = found!.off ? turnedBackOnBy(read, later) : undefined;
  if (back !== undefined) {
    readTurnedBackOn(read, key, because, back);
    return;
  }
  const config = preset === undefined ? everyFileOptions(read, key, found!, because, later) : undefined;
  read.decisions.push({ on: !found!.off, explicit: preset === undefined, because, ...(config === undefined ? {} : { config }) });
}

/** An entry for some files that turns the rule off: only a whole folder's files can start off. */
function readScopedOff(read: PortRead, key: string, entry: ScopedRule, because: string, after: readonly ScopedRule[]): void {
  const back = turnedBackOnBy(read, after);
  if (back === undefined && entry.folders !== undefined) {
    for (const folder of entry.folders) read.folders.push({ folder, on: false, because });
    return;
  }
  // A shared config's off for some files is how it fits those files, not worth a note.
  if (entry.preset !== undefined) return;
  read.notes.push(back === undefined
    ? `${read.port.ruleId}: ${because} turns it off only for some files; it stays on.`
    : `${read.port.ruleId}: ${because} turns it off, but ${entryText(read.eslint.file, key, back.value)} turns it back on for ${back.files}; it stays on.`);
}

/**
 * The options of an entry for some files that turns the rule on. A rule setting's config is for the whole repository,
 * so they are used only when no entry of the config's own for every file sets the rule: otherwise they would change
 * it for files they do not cover. A shared config's entry for every file gives a severity only, its options unknown.
 */
function scopedOptions(read: PortRead, key: string, because: string, options: readonly unknown[]): MappedOptions | undefined {
  const { eslint, port: { ruleId }, notes } = read;
  const own = Object.hasOwn(eslint.rules, key) && !eslint.presets.has(key);
  const forEvery = own ? severity(read, eslint.rules[key]) : undefined;
  if (forEvery === undefined) return mappedOptions(read, options);
  // An entry for every file with options of its own says so where it is read; one with a severity alone keeps the defaults.
  if (forEvery.options.length === 0) {
    notes.push(`${ruleId}: the options of ${because} are not used: ${entryText(eslint.file, key, eslint.rules[key])} sets the rule for every file, and Rerail sets options for the whole repository only.`);
  }
  return undefined;
}

/**
 * An entry for some files that turns the rule on: on in the whole folders it covers; on everywhere for narrower files
 * (it runs there), or where it sets options the port takes.
 */
function readScopedOn(read: PortRead, key: string, entry: ScopedRule, because: string, options: readonly unknown[]): void {
  for (const folder of entry.folders ?? []) read.folders.push({ folder, on: true, because });
  // A shared config's options are not mapped: it gives severities only.
  const config = options.length === 0 ? undefined : scopedOptions(read, key, because, options);
  if (entry.folders !== undefined && config === undefined) return;
  read.decisions.push({ on: true, explicit: entry.preset === undefined, because, ...(config === undefined ? {} : { config }) });
}

/** What the config says about `key` for some files; `after` are the entries for `key` that come after this one. */
function readScopedEntry(read: PortRead, key: string, entry: ScopedRule, after: readonly ScopedRule[]): void {
  const because = `${entryBecause(read, key, entry.value, entry.preset)} for ${entry.files}`;
  const found = severity(read, entry.value);
  if (skipUnused(read, unusedEntry(read, because, found, entry.shadowedBy), entry.preset, because)) return;
  if (found!.off) readScopedOff(read, key, entry, because, after);
  else readScopedOn(read, key, entry, because, found!.options);
}

/** A rule the config does not set for every file: off, as ESLint leaves it, unless something not read may set it. */
function unsetDecision(eslint: EslintRules): PortDecision {
  return eslint.unread === undefined
    ? { on: false, explicit: false, because: `${eslint.file} does not turn it on` }
    : { on: "unknown", because: `${eslint.file}: ${eslint.unread} is not read and may set it` };
}

/** What the config says about rule key `key` of the port. */
function readEslintKey(read: PortRead, key: string): void {
  const before = read.decisions.length;
  const unresolved = read.eslint.unresolved.get(key);
  if (unresolved !== undefined) {
    readUnresolvedKey(read, key, unresolved);
    return;
  }
  const later = read.eslint.scoped.filter(entry => entry.key === key);
  if (Object.hasOwn(read.eslint.rules, key)) readEveryFileEntry(read, key, later);
  for (const [index, entry] of later.entries()) readScopedEntry(read, key, entry, later.slice(index + 1));
  if (read.decisions.length === before) read.decisions.push(unsetDecision(read.eslint));
}

/** The verdict of the ESLint config on `port`, over the ESLint rules it ports. */
function eslintVerdict(eslint: EslintRules, port: Port, ruleCatalog: RuleCatalog): PortVerdict | undefined {
  const keys = new Set(port.suppress.eslint);
  if (keys.size === 0) return undefined;
  if (eslint.unreadable !== undefined) return { decision: { on: false, explicit: true, because: `${eslint.file} is not statically readable` } };
  const read: PortRead = { ruleCatalog, port, eslint, decisions: [], folders: [], notes: [] };
  for (const key of keys) readEslintKey(read, key);
  // Every key adds at least one decision (`readEslintKey`).
  return { decision: combineDecisions(read.decisions)!, folders: read.folders, notes: read.notes };
}

export const eslintReader: LinterReader = {
  linter: "eslint",
  kinds: ["eslint"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const eslint = readEslintRules(root, notes);
    return eslint === undefined ? undefined : { verdict: (port, ruleCatalog) => eslintVerdict(eslint, port, ruleCatalog) };
  },
};
