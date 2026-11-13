/**
 * What a repository's ESLint config (`eslint-rules.ts`) says about each port of ESLint rules: the verdict set-up
 * settles with the other linters'. A rule an entry turns on or off is decided explicitly (with its options mapped onto
 * the port's `config` where it declares them), one a recognised shared config's entry turns on or off implicitly. A
 * rule the config never sets is off, as ESLint leaves it, when Rerail reads the whole config, and unknown (the pack's
 * value stands) when something it takes and Rerail does not read may set it. An entry for a whole folder's files turns
 * the rule on or off in that folder; one for files narrower than a folder turns it on everywhere (it runs there) and
 * leaves an off with a note. A plugin rule is read under every id the config names it by (`ts/no-explicit-any` where
 * `plugins: { ts: tseslint.plugin }`), and runs where any is on; a key under a namespace Rerail cannot tell the plugin
 * of may be it, so where that key is on the rule is unknown. A flat config Rerail cannot read statically, and an entry
 * whose value needs the config run, leave the port's value to the pack.
 */
import { codeRuleDialectFamily, codeRuleDialectForPath } from "../../code-rule/code-rule-languages.js";
import type { CodeRuleDslValue } from "../../code-rule/code-rule.js";
import { stableJsonText } from "../../stable-json.js";
import type { RuleCatalog } from "../repository-model.js";
import { configureBundledCodeRule } from "../rule-config.js";
import { globPattern } from "../rule-pack-matcher.js";
import type { FilePatterns } from "./eslint-flat-config.js";
import { PLUGIN_NAMESPACES } from "./eslint-presets.js";
import { readEslintRules, type EslintRules, type ScopedRule, type UnreadEntry } from "./eslint-rules.js";
import { entryText, jsonObject, severityOf, type Severity } from "./jsonc-lint-config.js";
import { combineDecisions } from "./linter-decisions.js";
import { inFolder, type Declared, type FolderDecision, type LinterConfig, type LinterNote, type LinterReader, type MappedOptions, type Port, type PortDecision, type PortVerdict } from "./linter-config.js";
import { repositoryFiles } from "./repository-files.js";

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
  /** The repository's files the port runs on (its document's `scope`); undefined where Rerail cannot match its globs. */
  readonly files: () => readonly string[] | undefined;
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

/** The repository sets the rule, to what Rerail cannot tell without running the config: the pack's value stands. */
function readUnresolvedKey(read: PortRead, key: string, entry: UnreadEntry): void {
  const because = `${read.eslint.file}: ${JSON.stringify(key)} is not read (${entry.why})`;
  if (entry.shadowedBy !== undefined) read.notes.push(`${read.port.ruleId}: ${because} and is not used: ${replacedBy(entry.shadowedBy)}.`);
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

/** The entry every file gets: the config's for every file, or a later one for some files that holds every file the port runs on. */
interface EveryFile {
  readonly value: unknown;
  readonly preset?: string | undefined;
  readonly shadowedBy?: string | undefined;
}

/** What the config says about `key` for every file; `later` are the entries for whole folders after it. */
function readEveryFileEntry(read: PortRead, key: string, every: EveryFile, later: readonly ScopedRule[]): void {
  const { value, preset } = every;
  const because = entryBecause(read, key, value, preset);
  const found = severity(read, value);
  if (skipUnused(read, unusedEntry(read, entryText(read.eslint.file, key, value), found, every.shadowedBy), preset, because)) return;
  // Turned back on for whole folders: off elsewhere, and on there (`readScopedEntry`).
  if (found!.off && turnedBackOnBy(read, later) !== undefined) {
    read.decisions.push({ on: false, explicit: true, because });
    return;
  }
  const config = preset === undefined ? everyFileOptions(read, key, found!, because, later) : undefined;
  read.decisions.push({ on: !found!.off, explicit: preset === undefined, because, ...(config === undefined ? {} : { config }) });
}

/** Whether a folder in `folders` holds `folder` or is it. */
const holds = (folders: readonly string[], folder: string): boolean => folders.some(holder => folder === holder || folder.startsWith(`${holder}/`));

/** An entry for whole folders that turns the rule off there, but where a later entry for a folder holding one turns it back on. */
function readScopedOff(read: PortRead, entry: ScopedRule, because: string, after: readonly ScopedRule[]): void {
  const backOn = after.filter(later => severity(read, later.value)?.off === false).flatMap(later => later.folders!);
  for (const folder of entry.folders!) if (!holds(backOn, folder)) read.folders.push({ folder, on: false, explicit: true, because });
}

/**
 * An entry for whole folders that turns the rule on there. Its options are not used: a rule setting's config is for the
 * whole repository, where they would change the rule for files they do not cover.
 */
function readScopedOn(read: PortRead, entry: ScopedRule, because: string, options: readonly unknown[]): void {
  for (const folder of entry.folders!) read.folders.push({ folder, on: true, explicit: true, because });
  if (options.length > 0) read.notes.push(`${read.port.ruleId}: the options of ${because} are not used: Rerail sets options for the whole repository only.`);
}

/** What the config says about `key` for whole folders; `after` are the entries for whole folders that come after this one. */
function readScopedEntry(read: PortRead, key: string, entry: ScopedRule, after: readonly ScopedRule[]): void {
  const because = `${entryBecause(read, key, entry.value, entry.preset)} for ${entry.files}`;
  const found = severity(read, entry.value);
  if (skipUnused(read, unusedEntry(read, because, found, entry.shadowedBy), entry.preset, because)) return;
  if (found!.off) readScopedOff(read, entry, because, after);
  else readScopedOn(read, entry, because, found!.options);
}

/**
 * The options of an entry for only some of the files the port runs on that keeps the rule on as it stands. A rule
 * setting's config is for the whole repository, so they are used only where a shared config, which gives severities
 * alone, turns the rule on for every file; an entry of the config's own for every file decides the options.
 */
function readPartialOptions(read: PortRead, key: string, every: EveryFile, because: string, options: readonly unknown[]): void {
  if (options.length === 0) return;
  if (every.preset !== undefined) {
    read.decisions.push({ on: true, explicit: true, because, config: mappedOptions(read, options) });
    return;
  }
  // An entry for every file with options of its own says so where it is read; one with a severity alone keeps the defaults.
  if (severity(read, every.value)!.options.length === 0) {
    read.notes.push(`${read.port.ruleId}: the options of ${because} are not used: ${entryText(read.eslint.file, key, every.value)} sets the rule for every file, and Rerail sets options for the whole repository only.`);
  }
}

/**
 * Why an entry for only some of the files the port runs on, not whole folders, leaves the rule on for some of them and
 * off for others, which a rule setting cannot say, if it does: it does unless it sets the rule as it stands, in `states`,
 * for every file and in every folder. An entry not used may do either.
 */
function partialMix(read: PortRead, key: string, entry: ScopedRule, every: EveryFile | undefined, states: readonly (boolean | "unknown")[]): string | undefined {
  const because = `${entryBecause(read, key, entry.value, entry.preset)} for ${entry.files}`;
  const found = severity(read, entry.value);
  const unused = unusedEntry(read, because, found, entry.shadowedBy);
  if (unused === undefined && states.every(state => state === !found!.off)) {
    // Kept on, it is on for every file, which `every` turns it on for.
    if (!found!.off) readPartialOptions(read, key, every!, because, found!.options);
    return undefined;
  }
  // A shared config's entry for some files is how it fits those files, not the person's to change.
  if (entry.preset === undefined) read.notes.push(unused ?? `${read.port.ruleId}: ${because} turns it ${found!.off ? "off" : "on"} for only some of the files it runs on; it keeps the pack's value.`);
  return because;
}

/** A rule the config does not set for every file: off, as ESLint leaves it, unless something not read may set it. */
function unsetDecision(eslint: EslintRules): PortDecision {
  return eslint.unread === undefined
    ? { on: false, explicit: false, because: `${eslint.file} does not turn it on` }
    : { on: "unknown", because: `${eslint.file}: ${eslint.unread} is not read and may set it` };
}

/** How an entry for some files bears on the port: on none of its files, on all of them, on whole folders, or on only some. */
type Bearing = "none" | "every" | "folders" | "some";

/** How an entry matching `matched` of the port's `files` bears on it. */
function shareBearing(matched: number, files: number): Bearing {
  if (matched === 0) return "none";
  return matched === files ? "every" : "some";
}

/** How `entry` bears on the files the port runs on; `some` where Rerail cannot match its globs. */
function bearingOf(read: PortRead, entry: ScopedRule): Bearing {
  if (entry.folders !== undefined) return "folders";
  const files = entry.patterns === undefined ? undefined : read.files();
  const matches = files === undefined ? undefined : patternsMatcher(entry.patterns!);
  return matches === undefined ? "some" : shareBearing(files!.filter(matches).length, files!.length);
}

/** The config's own entry every file gets for `key`, if it has one. */
function everyFileEntry(read: PortRead, key: string): EveryFile | undefined {
  const { eslint } = read;
  return Object.hasOwn(eslint.rules, key) ? { value: eslint.rules[key], preset: eslint.presets.get(key), shadowedBy: eslint.shadowed.get(key) } : undefined;
}

/** The rule's states read since `start`: for every file (unset where nothing set it), and in each folder. */
function statesSince(read: PortRead, start: { readonly decisions: number; readonly folders: number }): (boolean | "unknown")[] {
  const whole = read.decisions.slice(start.decisions);
  return [...(whole.length > 0 ? whole : [unsetDecision(read.eslint)]), ...read.folders.slice(start.folders)].map(decision => decision.on);
}

/**
 * What the config's entries say about `key`. An entry for some files that holds none of the files the port runs on
 * does not bear on it; one that holds all of them is, for the port, an entry every file gets; one that holds only some
 * and sets the rule otherwise than it stands leaves it to the pack.
 */
function readEntries(read: PortRead, key: string): void {
  const start = { decisions: read.decisions.length, folders: read.folders.length };
  const scoped = read.eslint.scoped.filter(entry => entry.key === key).map(entry => ({ entry, bearing: bearingOf(read, entry) }));
  const last = scoped.findLastIndex(({ bearing }) => bearing === "every");
  const every = last < 0 ? everyFileEntry(read, key) : scoped[last]!.entry;
  const later = scoped.slice(last + 1);
  const folders = later.filter(({ bearing }) => bearing === "folders").map(({ entry }) => entry);
  if (every !== undefined) readEveryFileEntry(read, key, every, folders);
  for (const [index, entry] of folders.entries()) readScopedEntry(read, key, entry, folders.slice(index + 1));
  const states = statesSince(read, start);
  const mixed = later.filter(({ bearing }) => bearing === "some").map(({ entry }) => partialMix(read, key, entry, every, states)).find(because => because !== undefined);
  if (mixed === undefined) return;
  read.decisions.splice(start.decisions);
  read.folders.splice(start.folders);
  read.decisions.push({ on: "unknown", because: mixed });
}

/** What the config says about rule key `key` of the port. */
function readEslintKey(read: PortRead, key: string): void {
  const before = read.decisions.length;
  const unresolved = read.eslint.unresolved.get(key);
  if (unresolved !== undefined) readUnresolvedKey(read, key, unresolved);
  else readEntries(read, key);
  if (read.decisions.length === before) read.decisions.push(unsetDecision(read.eslint));
}

/** What the config says about some rule keys: for every file, and per whole folder. */
interface KeyReading {
  readonly decision: PortDecision;
  readonly folders: readonly FolderDecision[];
}

/** What the config says about `keys`, read as one port's rule keys. */
function readKeys(base: Omit<PortRead, "decisions" | "folders">, keys: readonly string[]): KeyReading {
  const read: PortRead = { ...base, decisions: [], folders: [] };
  for (const key of keys) readEslintKey(read, key);
  // Every key adds at least one decision (`readEslintKey`).
  return { decision: combineDecisions(read.decisions)!, folders: read.folders };
}

/** A rule id as ESLint splits it (`parseRuleId`, lib/config/config.js): a plugin rule's namespace (none for a core rule) and its name. */
function ruleIdParts(key: string): { readonly namespace: string | undefined; readonly name: string } {
  if (!key.includes("/")) return { namespace: undefined, name: key };
  const split = key.startsWith("@") ? key.lastIndexOf("/") : key.indexOf("/");
  return { namespace: key.slice(0, split), name: key.slice(split + 1) };
}

/**
 * The namespace of plugin rules a namespace of the config names: the one its `plugins` bind it to, else itself (a
 * recognised plugin's own, which its shared configs bind); undefined where Rerail cannot tell which plugin it is
 * (`plugins` bind it to one Rerail does not recognise, or something unread may bind it).
 */
function pluginNamespaceOf(eslint: EslintRules, namespace: string): string | undefined {
  if (eslint.namespaces.has(namespace)) return eslint.namespaces.get(namespace);
  return eslint.namespacesUnread === undefined || PLUGIN_NAMESPACES.has(namespace) ? namespace : undefined;
}

/** How config key `other` names plugin rule `name` of `namespace`: the same rule, possibly the same rule, or another. */
function relation(eslint: EslintRules, namespace: string, name: string, other: string): "same" | "maybe" | undefined {
  const parts = ruleIdParts(other);
  if (parts.namespace === undefined || parts.name !== name) return undefined;
  const plugin = pluginNamespaceOf(eslint, parts.namespace);
  if (plugin === undefined) return "maybe";
  return plugin === namespace ? "same" : undefined;
}

/**
 * The config's other keys for port key `key`'s plugin rule: under each namespace that names its plugin
 * (`ts/no-explicit-any` with `plugins: { ts: tseslint.plugin }`), and under each Rerail cannot tell the plugin of. A
 * core rule has one name.
 */
function otherKeys(eslint: EslintRules, key: string): { readonly same: string[]; readonly maybe: string[] } {
  const { namespace, name } = ruleIdParts(key);
  const others = new Set([...Object.keys(eslint.rules), ...eslint.scoped.map(entry => entry.key), ...eslint.unresolved.keys()]);
  others.delete(key);
  const related = namespace === undefined ? [] : [...others].map(other => ({ other, as: relation(eslint, namespace, name, other) }));
  return { same: related.filter(({ as }) => as === "same").map(({ other }) => other), maybe: related.filter(({ as }) => as === "maybe").map(({ other }) => other) };
}

/** A key that may name the rule: where it is on, the rule may run; where it is off, it says nothing of the rule. */
function possibly(reading: KeyReading, file: string, key: string): KeyReading {
  const because = `${file}: ${JSON.stringify(key)} may name this rule: Rerail cannot tell which plugin its namespace is`;
  return {
    decision: reading.decision.on === false ? reading.decision : { on: "unknown", because },
    folders: reading.folders.map(folder => (folder.on === false ? folder : { ...folder, on: "unknown", because })),
  };
}

/** How surely a decision runs the rule, surest first: ESLint runs a rule under every id it is on under. */
function runRank(decision: { readonly on: boolean | "unknown"; readonly explicit?: boolean }): number {
  if (decision.on === "unknown") return 2;
  const rank = decision.explicit === false ? 1 : 0;
  return decision.on ? rank : 3 + rank;
}

function mostRunning<D extends { readonly on: boolean | "unknown"; readonly explicit?: boolean }>(decisions: readonly D[]): D {
  return decisions.reduce((best, decision) => (runRank(decision) < runRank(best) ? decision : best));
}

/** One reading's state in `folder`: its decisions there (an off winning, as settling has it), else its decision for every file. */
function folderState(reading: KeyReading, folder: string): FolderDecision | PortDecision {
  const here = reading.folders.filter(decision => decision.folder === folder);
  return here.find(decision => decision.on === false) ?? here.find(decision => decision.on === true) ?? here[0] ?? reading.decision;
}

/** Per folder any reading decides, the state of the rule there over all its ids. */
function sameRuleFolders(readings: readonly KeyReading[]): FolderDecision[] {
  const folders = [...new Set(readings.flatMap(reading => reading.folders.map(decision => decision.folder)))];
  return folders.map(folder => inFolder(folder, mostRunning(readings.map(reading => folderState(reading, folder)))));
}

/**
 * `folders` less those in a folder that holds an ESLint config of its own, where the root config does not decide, and
 * each such folder unknown: the pack's value stands there.
 */
function nestedFolders(eslint: EslintRules, folders: readonly FolderDecision[]): FolderDecision[] {
  const inNested = (folder: string): boolean => eslint.nested.some(nested => folder === nested.folder || folder.startsWith(`${nested.folder}/`));
  return [
    ...folders.filter(decision => !inNested(decision.folder)),
    ...eslint.nested.map(({ folder, file }): FolderDecision => ({ folder, on: "unknown", because: `${file} is not read` })),
  ];
}

/**
 * The verdict of the ESLint config on `port`, over the ESLint rules it ports, under their own names and under every
 * other id the config names them by: the rule runs where any of them is on. `files` are the repository's files the
 * port runs on.
 */
function eslintVerdict(eslint: EslintRules, port: Port, ruleCatalog: RuleCatalog, files: () => readonly string[] | undefined): PortVerdict | undefined {
  const keys = [...new Set(port.suppress.eslint)];
  if (keys.length === 0) return undefined;
  if (eslint.unreadable !== undefined) return { decision: { on: "unknown", because: `${eslint.file} is not statically readable` } };
  const base = { ruleCatalog, port, eslint, files, notes: [] as string[] };
  const own = readKeys(base, keys);
  const others = keys.map(key => otherKeys(eslint, key));
  const readings = [
    own,
    ...others.flatMap(other => other.same).map(key => readKeys(base, [key])),
    ...others.flatMap(other => other.maybe).map(key => possibly(readKeys(base, [key]), eslint.file, key)),
  ];
  const decision = readings.length === 1 ? own.decision : mostRunning(readings.map(reading => reading.decision));
  const folders = readings.length === 1 ? own.folders : sameRuleFolders(readings);
  return { decision, folders: nestedFolders(eslint, folders), notes: base.notes };
}

/**
 * A matcher of the files `patterns` holds, reading globs as the pack matcher does (`globPattern`); undefined where a
 * glob does not read.
 */
function patternsMatcher(patterns: FilePatterns): ((file: string) => boolean) | undefined {
  try {
    const groups = patterns.files.map(globs => globPattern(globs));
    const ignored = globPattern(patterns.ignores);
    return file => groups.every(group => group.test(file)) && !ignored.test(file);
  } catch {
    return undefined;
  }
}

/** The families of the files ESLint lints: JavaScript, TypeScript and their JSX, and Vue's single-file components. */
const ESLINT_FAMILIES: readonly string[] = ["ecma", "vue"];

/** The repository's files ESLint lints that `scope` holds; undefined where its globs do not read. */
function filesInScope(root: string, scope: Port["scope"]): readonly string[] | undefined {
  const inScope = patternsMatcher({ files: [scope?.include ?? ["**/*"]], ignores: scope?.exclude ?? [] });
  return inScope === undefined ? undefined : repositoryFiles(root).filter(file => {
    const dialect = codeRuleDialectForPath(file);
    return dialect !== undefined && ESLINT_FAMILIES.includes(codeRuleDialectFamily(dialect)) && inScope(file);
  });
}

/** Per port, the repository's files it runs on (`filesInScope`), listed once per scope when first asked for. */
function portFiles(root: string): (port: Port) => () => readonly string[] | undefined {
  const byScope = new Map<string, readonly string[] | undefined>();
  return port => () => {
    const key = JSON.stringify(port.scope ?? null);
    if (!byScope.has(key)) byScope.set(key, filesInScope(root, port.scope));
    return byScope.get(key);
  };
}

export const eslintReader: LinterReader = {
  linter: "eslint",
  kinds: ["eslint"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const eslint = readEslintRules(root, notes);
    const files = portFiles(root);
    return eslint === undefined ? undefined : { verdict: (port, ruleCatalog) => eslintVerdict(eslint, port, ruleCatalog, files(port)) };
  },
};
