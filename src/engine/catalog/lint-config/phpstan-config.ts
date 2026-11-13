/**
 * PHPStan's config at the repository root, read as data (`neon-format.ts`) for the php pack's PHPStan ports, as
 * PHPStan 2.2.16 reads it (phpstan-src@08e3736, the release `packs/php/conformance.json` names).
 *
 * The rule level decides which rules run: each rule class is registered by the level its `#[RegisteredRule(level: N)]`
 * names, and level N runs every rule of levels 0 to N. The level is `parameters.level` (0 to 10, or `max`), or the
 * level of a `conf/config.level<N>.neon` the config includes through the phar's alias (`phar://phpstan.phar/conf/…`:
 * the Composer package's own `conf/` holds `bleedingEdge.neon` alone); a config with neither runs at the `--level` its
 * command line passes, which Rerail does not read. `rules:` and services tagged `phpstan.rules.rule` register rules at
 * every level (src/DependencyInjection/RulesExtension.php): the port's own rule class turns it on; any other class may
 * report its identifier, so a rule the level leaves off keeps the pack's value. An `ignoreErrors` entry naming an
 * identifier with no message, count or path turns its rule off; with a path that is a folder of the repository, off in
 * that folder. Repository files the config includes are read in turn; any other include (an extension's `rules.neon`,
 * a file outside the repository) may register rules or ignore errors, so a rule only the level decides keeps the
 * pack's value.
 */
import { posix } from "node:path";
import { readFormat } from "./formats/lint-config-formats.js";
import {
  NOT_A_FILE,
  readRootFile,
  type FolderDecision,
  type LinterConfig,
  type LinterNote,
  type LinterReader,
  type Port,
  type PortDecision,
  type PortVerdict,
} from "./linter-config.js";
import { isNeonArray, neonEntry, parseNeon, type NeonItem, type NeonValue } from "./formats/neon-format.js";
import { isRepositoryFolder, repositoryPath } from "./repository-paths.js";

/** The config files PHPStan looks for at the root, in its order (src/Command/CommandHelper.php l.195-202); it uses the first present. */
const PHPSTAN_FILES = [".phpstan.neon", "phpstan.neon", ".phpstan.neon.dist", "phpstan.neon.dist", ".phpstan.dist.neon", "phpstan.dist.neon"];

/**
 * Per identifier the php pack ports, the rule class the port ports and its level, as its `#[RegisteredRule(level: N)]`
 * names it at phpstan-src@08e3736: PrintfParametersRule (src/Rules/Functions/PrintfParametersRule.php l.23),
 * DuplicateKeysInLiteralArraysRule (src/Rules/Arrays/DuplicateKeysInLiteralArraysRule.php l.31), DefinedVariableRule
 * (src/Rules/Variables/DefinedVariableRule.php l.24), IfConstantConditionRule (src/Rules/Comparison/
 * IfConstantConditionRule.php l.19), RandomIntParametersRule (src/Rules/Functions/RandomIntParametersRule.php l.23)
 * and MissingFunctionReturnTypehintRule (src/Rules/Functions/MissingFunctionReturnTypehintRule.php l.20). Other rules
 * report some of these identifiers too, at other levels; a port is the one rule it ports.
 */
const IDENTIFIER_RULES: Readonly<Record<string, { readonly level: number; readonly rule: string }>> = {
  "argument.sprintf": { level: 0, rule: "PHPStan\\Rules\\Functions\\PrintfParametersRule" },
  "argument.sscanf": { level: 0, rule: "PHPStan\\Rules\\Functions\\PrintfParametersRule" },
  "array.duplicateKey": { level: 0, rule: "PHPStan\\Rules\\Arrays\\DuplicateKeysInLiteralArraysRule" },
  "variable.undefined": { level: 0, rule: "PHPStan\\Rules\\Variables\\DefinedVariableRule" },
  "if.alwaysFalse": { level: 4, rule: "PHPStan\\Rules\\Comparison\\IfConstantConditionRule" },
  "if.alwaysTrue": { level: 4, rule: "PHPStan\\Rules\\Comparison\\IfConstantConditionRule" },
  "argument.type": { level: 5, rule: "PHPStan\\Rules\\Functions\\RandomIntParametersRule" },
  "missingType.return": { level: 6, rule: "PHPStan\\Rules\\Functions\\MissingFunctionReturnTypehintRule" },
};

/** The highest level, which `max` names (conf/config.levelmax.neon includes config.level10.neon). */
const MAX_LEVEL = 10;
/** A level as `parameters.level` or a level config's name writes it. */
const LEVEL_TEXT = /^(?:10|[0-9]|max)$/u;
/**
 * An include of a config PHPStan ships: a level config or bleeding edge through the phar's alias, or bleeding edge
 * through `%rootDir%` or the Composer package, whose `conf/` holds no level config (including one there stops PHPStan).
 */
const SHIPPED_CONFIG = /^(?:phar:\/\/phpstan\.phar\/conf\/(?:bleedingEdge\.neon|config\.level(10|[0-9]|max)\.neon)|(?:%rootDir%|(?:.*\/)?vendor\/phpstan\/phpstan)\/conf\/bleedingEdge\.neon)$/u;
/** The tag that registers a service as a rule (`LazyRegistry::RULE_TAG`). */
const RULE_TAG = "phpstan.rules.rule";
/** How many configs deep includes are followed; nette/di has no limit, but a chain this long is not one a person wrote. */
const MAX_INCLUDE_DEPTH = 8;
const MESSAGE_KEYS = ["message", "messages", "rawMessage", "rawMessages"];

/** One `ignoreErrors` entry that names identifiers. */
interface PhpstanIgnore {
  /** The entry, for the person: `phpstan.neon: ignoreErrors identifier missingType.return`. */
  readonly because: string;
  readonly identifiers: readonly string[];
  /** Why it ignores only some of the identifiers' errors (a message, a count); undefined when it ignores them all. */
  readonly partial: string | undefined;
  /** The folders it is limited to; undefined for every file. */
  readonly folders: readonly string[] | undefined;
  /** The paths it is limited to that are not folders of the repository. */
  readonly otherPaths: readonly string[];
}

/** What one config file sets itself, its includes apart. */
interface PhpstanLayer {
  readonly file: string;
  /** `parameters.level`, as written. */
  readonly level?: NeonValue;
  /** The level of a level config PHPStan ships (the include itself is the layer). */
  readonly levelConfig?: number;
  readonly customRuleset?: boolean;
  readonly ignores: readonly PhpstanIgnore[];
  /** `ignoreErrors!`: its entries replace those of the configs before it. */
  readonly replacesIgnores: boolean;
}

/** The config files read, from the lowest priority to the highest, as nette/di merges them, and what was not read. */
interface PhpstanRead {
  readonly layers: PhpstanLayer[];
  /** Includes not read: the include and why, for the person. */
  readonly unread: { readonly what: string; readonly why: string }[];
  /** The rule classes `rules:` and tagged services register at every level, as written; `undefined` for a class not written as a name. */
  readonly registered: { readonly rule: string | undefined; readonly because: string }[];
}

/** The level the config runs at, or why Rerail cannot tell it. */
type LevelRead = { readonly value: number; readonly because: string } | { readonly unknown: string };

/** A level as written, as a number; undefined when PHPStan has no such level. */
function levelValue(written: NeonValue | undefined): number | undefined {
  const text = typeof written === "number" || typeof written === "string" ? String(written) : "";
  if (!LEVEL_TEXT.test(text)) return undefined;
  return text === "max" ? MAX_LEVEL : Number(text);
}

/** The strings of a string or a list of strings; undefined for anything else. */
function stringsOf(value: NeonValue | undefined): string[] | undefined {
  if (typeof value === "string") return [value];
  if (!isNeonArray(value)) return undefined;
  const strings = value.items.map(item => item.value);
  return strings.every(item => typeof item === "string") ? strings as string[] : undefined;
}

/**
 * Path `written` in config `file` as a path relative to the root: relative to the file's folder, or to the root through
 * `%currentWorkingDirectory%`, which PHPStan expands (NeonAdapter::expandKnownParameters); undefined for another
 * parameter, an absolute path or one outside the repository.
 */
function configPath(file: string, written: string): string | undefined {
  const fromRoot = /^%currentWorkingDirectory%(?:\/|$)/u.exec(written);
  const rest = fromRoot === null ? written : written.slice(fromRoot[0].length);
  if (rest.includes("%")) return undefined;
  return repositoryPath(fromRoot === null ? posix.dirname(file) : "", rest);
}

/** A wildcard of the `fnmatch` patterns PHPStan's paths take. */
const PATH_WILDCARD = /[*?[]/u;

/** A path an `ignoreErrors` entry is limited to: a folder of the repository, or not. */
function ignoredPath(root: string, file: string, written: string): { folder: string } | { other: string } {
  const path = PATH_WILDCARD.test(written) ? undefined : configPath(file, written);
  return path !== undefined && path !== "" && isRepositoryFolder(root, path) ? { folder: path } : { other: written };
}

/** Why an entry ignores only some of its identifiers' errors: a message it matches, or a count; undefined when it ignores them all. */
function partialOf(entry: NeonValue): string | undefined {
  const message = MESSAGE_KEYS.find(key => neonEntry(entry, key) !== undefined);
  if (message !== undefined) return `the errors its ${message} matches`;
  return neonEntry(entry, "count") === undefined ? undefined : "a count of its errors";
}

/** An `ignoreErrors` entry that is a mapping and names identifiers, as read; undefined for any other entry. */
function ignoreOf(root: string, file: string, entry: NeonValue): PhpstanIgnore | undefined {
  const identifiers = stringsOf(neonEntry(entry, "identifier")) ?? stringsOf(neonEntry(entry, "identifiers")) ?? [];
  if (identifiers.length === 0) return undefined;
  const written = stringsOf(neonEntry(entry, "path")) ?? stringsOf(neonEntry(entry, "paths"));
  const paths = (written ?? []).map(path => ignoredPath(root, file, path));
  return {
    because: `${file}: ignoreErrors ${identifiers.length === 1 ? "identifier" : "identifiers"} ${identifiers.join(", ")}`,
    identifiers,
    partial: partialOf(entry),
    folders: written === undefined ? undefined : paths.flatMap(path => ("folder" in path ? [path.folder] : [])),
    otherPaths: paths.flatMap(path => ("other" in path ? [path.other] : [])),
  };
}

/** The value of `key` in `parameters`, or of `key!`, which replaces what the configs before it set. */
function parameter(parameters: NeonValue | undefined, key: string): { value: NeonValue | undefined; replaces: boolean } {
  const replacing = neonEntry(parameters, `${key}!`);
  return replacing === undefined ? { value: neonEntry(parameters, key), replaces: false } : { value: replacing, replaces: true };
}

/** The `ignoreErrors` of one file, noting the entries Rerail does not read. */
function ignoresOf(root: string, file: string, entries: NeonValue | undefined, notes: LinterNote[]): PhpstanIgnore[] {
  if (entries === undefined || entries === null) return [];
  if (!isNeonArray(entries)) {
    notes.push({ linter: "phpstan", text: `${file}: its ignoreErrors are not read: they are not a list.` });
    return [];
  }
  const read = entries.items.map(item => ignoreOf(root, file, item.value));
  const unread = read.filter(ignore => ignore === undefined).length;
  if (unread > 0) notes.push({ linter: "phpstan", text: `${file}: ${unread} of its ignoreErrors ${unread === 1 ? "entry names" : "entries name"} no identifier; Rerail reads the identifiers an entry names, not the messages it matches.` });
  return read.filter((ignore): ignore is PhpstanIgnore => ignore !== undefined);
}

/** The layer config file `file` (its text parsed) sets itself. */
function layerOf(root: string, file: string, config: NeonValue, notes: LinterNote[]): PhpstanLayer {
  const parameters = neonEntry(config, "parameters");
  const ignoreErrors = parameter(parameters, "ignoreErrors");
  const level = parameter(parameters, "level").value;
  const customRuleset = parameter(parameters, "customRulesetUsed").value === true;
  return {
    file,
    ...level === undefined ? {} : { level },
    ...customRuleset ? { customRuleset } : {},
    ignores: ignoresOf(root, file, ignoreErrors.value, notes),
    replacesIgnores: ignoreErrors.replaces,
  };
}

/** What include `written` of config `file` is: a config PHPStan ships (with its level, if a level config), a repository file to read, or why it is not read. */
function includeOf(file: string, written: NeonValue): { shipped: number | undefined } | { path: string } | { unread: string } {
  if (typeof written !== "string") return { unread: "it is not a path" };
  const shipped = SHIPPED_CONFIG.exec(written);
  if (shipped !== null) return { shipped: levelValue(shipped[1]) };
  const path = configPath(file, written);
  if (path === undefined) return { unread: "it is outside the repository or names a parameter" };
  if (/(?:^|\/)vendor\//u.test(path)) return { unread: "it is an installed package's config" };
  return path.endsWith(".php") ? { unread: "it is PHP, which Rerail never runs" } : { path };
}

/** Included config `file` and the configs it includes, added to `read`; `shown` is the include, for the person. */
function readIncludedFile(root: string, file: string, shown: string, depth: number, read: PhpstanRead, notes: LinterNote[]): void {
  const found = readRootFile(root, file);
  if (found.present && found.text !== undefined) readText(root, file, found.text, depth, read, notes);
  else read.unread.push({ what: shown, why: found.present ? NOT_A_FILE.replace(/\.$/u, "") : "it does not exist" });
}

/** Whether `tags` (a list of tag names, or a mapping keyed by them) holds the rule tag. */
function tagsRule(tags: NeonValue | undefined): boolean {
  return isNeonArray(tags) && tags.items.some(item => item.value === RULE_TAG || item.key === RULE_TAG);
}

/** The class a tagged service definition names (`class`, `factory` or `type`), when it names one as a name. */
function serviceClass(service: NeonValue): string | undefined {
  const named = ["class", "factory", "type"].map(key => neonEntry(service, key)).find(value => value !== undefined);
  return typeof named === "string" ? named : undefined;
}

/**
 * The rule classes config `file` registers at every level: its `rules:`, and its services tagged as rules; a
 * `conditionalTags:` rule, registered when a parameter Rerail does not evaluate is true, as any class.
 */
function registeredOf(file: string, config: NeonValue): PhpstanRead["registered"] {
  const items = (key: string): readonly NeonItem[] => {
    const value = neonEntry(config, key);
    return isNeonArray(value) ? value.items : [];
  };
  return [
    ...items("rules").map(item => ({ rule: typeof item.value === "string" ? item.value : undefined, because: `${file}: rules` })),
    ...items("services").filter(item => tagsRule(neonEntry(item.value, "tags"))).map(item => ({ rule: serviceClass(item.value), because: `${file}: services tagged ${RULE_TAG}` })),
    ...items("conditionalTags").filter(item => tagsRule(item.value)).map(item => ({ rule: undefined, because: `${file}: conditionalTags ${item.key ?? ""}` })),
  ];
}

/** Config file `file`, its includes first, in order, as nette/di merges them; false (with a note) when it is not NEON. */
function readText(root: string, file: string, text: string, depth: number, read: PhpstanRead, notes: LinterNote[]): boolean {
  const parsed = readFormat(() => parseNeon(text));
  if (!parsed.ok) {
    notes.push({ linter: "phpstan", text: `${file} is not read: ${parsed.why}.` });
    return false;
  }
  const config = parsed.value;
  const includes = neonEntry(config, "includes");
  for (const item of isNeonArray(includes) ? includes.items : []) readInclude(root, file, item.value, depth, read, notes);
  read.layers.push(layerOf(root, file, config, notes));
  read.registered.push(...registeredOf(file, config));
  return true;
}

function readInclude(root: string, file: string, written: NeonValue, depth: number, read: PhpstanRead, notes: LinterNote[]): void {
  const include = includeOf(file, written);
  const what = `${file}'s include ${JSON.stringify(written)}`;
  if ("shipped" in include) {
    if (include.shipped !== undefined) read.layers.push({ file: `${file}: includes ${String(written)}`, levelConfig: include.shipped, ignores: [], replacesIgnores: false });
  } else if ("unread" in include) read.unread.push({ what, why: include.unread });
  else if (depth >= MAX_INCLUDE_DEPTH) read.unread.push({ what, why: `includes are followed ${MAX_INCLUDE_DEPTH} deep` });
  else readIncludedFile(root, include.path, what, depth + 1, read, notes);
}

/** The level `parameters.level` sets, if any config sets it (the last read wins, as nette/di merges them), with an included level config's. */
function levelSet(read: PhpstanRead): LevelRead | undefined {
  const set = read.layers.findLast(layer => layer.level !== undefined);
  if (set === undefined) return undefined;
  const value = levelValue(set.level);
  const config = read.layers.findLast(layer => layer.levelConfig !== undefined);
  if (value === undefined) return { unknown: `${set.file}: level ${JSON.stringify(set.level)} is not a level PHPStan has` };
  if (config !== undefined && config.levelConfig !== value) return { unknown: `${set.file} sets level ${String(set.level)} and ${config.file}` };
  return { value, because: `${set.file}: level ${String(set.level)}` };
}

/** The level the read configs run at: `parameters.level`, or an included level config's; or why Rerail cannot tell it. */
function levelOf(read: PhpstanRead): LevelRead {
  const set = levelSet(read);
  if (set !== undefined) return set;
  const config = read.layers.findLast(layer => layer.levelConfig !== undefined);
  if (config?.levelConfig !== undefined) return { value: config.levelConfig, because: config.file };
  if (read.layers.some(layer => layer.customRuleset === true)) return { unknown: "the config is a custom ruleset (customRulesetUsed), whose rules Rerail does not read" };
  return { unknown: "the config sets no level, so PHPStan takes it from --level on its command line, which Rerail does not read" };
}

/** The ignores in force: those after the last `ignoreErrors!`, which replaces the ones before it. */
function ignoresInForce(layers: readonly PhpstanLayer[]): PhpstanIgnore[] {
  const from = layers.findLastIndex(layer => layer.replacesIgnores);
  return layers.slice(Math.max(from, 0)).flatMap(layer => layer.ignores);
}

/** A registered rule class as PHPStan matches it: no leading `\`, case-insensitively. */
const className = (written: string): string => written.replace(/^\\/u, "").toLowerCase();

/** The config read at the root: the level, the ignores, the rules registered at every level, and what may set the ports that Rerail does not read. */
class PhpstanConfig implements LinterConfig {
  readonly #level: LevelRead;
  readonly #ignores: readonly PhpstanIgnore[];
  readonly #unread: PhpstanRead["unread"];
  readonly #registered: PhpstanRead["registered"];

  constructor(read: PhpstanRead) {
    this.#level = levelOf(read);
    this.#ignores = ignoresInForce(read.layers);
    this.#unread = read.unread;
    this.#registered = read.registered;
  }

  verdict(port: Port): PortVerdict | undefined {
    const identifiers = port.suppress.phpstan;
    if (identifiers.length === 0) return undefined;
    const notes: string[] = [];
    const naming = this.#ignores.filter(ignore => ignore.identifiers.some(identifier => identifiers.includes(identifier)));
    for (const ignore of naming) notePartialIgnore(port, ignore, notes);
    const off = everyFileOff(naming, identifiers);
    const decision = off ?? this.#configDecision(identifiers);
    const folders = folderDecisions(naming, identifiers);
    return { decision, ...folders.length > 0 ? { folders } : {}, ...notes.length > 0 ? { notes } : {} };
  }

  /** What the config decides for a port of `identifiers` it does not ignore, unless an include Rerail does not read may decide it. */
  #configDecision(identifiers: readonly string[]): PortDecision {
    const rules = identifiers.map(identifier => IDENTIFIER_RULES[identifier]);
    const unknownIdentifier = identifiers.find((_, index) => rules[index] === undefined);
    if (unknownIdentifier !== undefined) return { on: "unknown", because: `the level PHPStan reports ${unknownIdentifier} from is not known to Rerail` };
    const unread = this.#unread[0];
    if (unread !== undefined) return { on: "unknown", because: `${unread.what}, which Rerail does not read, may register or ignore it` };
    const known = rules as readonly { level: number; rule: string }[];
    return this.#registeredOn(known) ?? this.#levelDecision(identifiers, Math.max(...known.map(rule => rule.level)));
  }

  /** On where `rules:` or a tagged service registers each of the port's own rule classes, whatever the level. */
  #registeredOn(rules: readonly { rule: string }[]): PortDecision | undefined {
    const own = rules.map(({ rule }) => this.#registered.find(registered => registered.rule !== undefined && className(registered.rule) === className(rule)));
    if (own.some(registered => registered === undefined)) return undefined;
    return { on: true, explicit: true, because: [...new Set(own.map(registered => `${registered!.because} ${registered!.rule!}`))].join("; ") };
  }

  /** What the level decides for a port PHPStan reports from level `from`; a rule another class registers may report it below. */
  #levelDecision(identifiers: readonly string[], from: number): PortDecision {
    if ("unknown" in this.#level) return { on: "unknown", because: this.#level.unknown };
    if (this.#level.value >= from) return { on: true, explicit: false, because: this.#level.because };
    const other = this.#registered[0];
    if (other !== undefined) return { on: "unknown", because: `${other.because} ${other.rule ?? "a rule"} registers a rule Rerail does not read, which may report ${identifiers.join(", ")}` };
    return { on: false, explicit: false, because: `${this.#level.because} does not turn it on (PHPStan reports ${identifiers.join(", ")} from level ${from})` };
  }
}

/** The explicit off when entries for every file ignore every identifier of the port. */
function everyFileOff(naming: readonly PhpstanIgnore[], identifiers: readonly string[]): PortDecision | undefined {
  const whole = naming.filter(ignore => ignore.folders === undefined && ignore.partial === undefined);
  if (!identifiers.every(identifier => whole.some(ignore => ignore.identifiers.includes(identifier)))) return undefined;
  return { on: false, explicit: true, because: whole.map(ignore => ignore.because).join("; ") };
}

/** Off in each folder an entry ignores every error of every identifier of the port in. */
function folderDecisions(naming: readonly PhpstanIgnore[], identifiers: readonly string[]): FolderDecision[] {
  return naming
    .filter(ignore => ignore.partial === undefined && identifiers.every(identifier => ignore.identifiers.includes(identifier)))
    .flatMap(ignore => (ignore.folders ?? []).map(folder => ({ folder, on: false, explicit: true, because: `${ignore.because} in path ${folder}` })));
}

/** Notes what an entry naming the port says that does not turn it off for every file or a whole folder. */
function notePartialIgnore(port: Port, ignore: PhpstanIgnore, notes: string[]): void {
  if (ignore.partial !== undefined) notes.push(`${port.ruleId}: ${ignore.because} ignores only ${ignore.partial}, so it does not turn the rule off.`);
  for (const path of ignore.otherPaths) notes.push(`${port.ruleId}: ${ignore.because} ignores it in ${JSON.stringify(path)}, which is not a folder of the repository; Rerail sets rules for whole folders only.`);
}

/** The config file PHPStan uses at the root, with its text; undefined (with a note when present but not read) otherwise. */
function usedConfig(root: string, notes: LinterNote[]): { file: string; text: string } | undefined {
  const present = PHPSTAN_FILES.filter(name => readRootFile(root, name).present);
  const file = present[0];
  if (file === undefined) return undefined;
  for (const name of present.slice(1)) notes.push({ linter: "phpstan", text: `${name} is not read: PHPStan uses ${file} instead.` });
  const found = readRootFile(root, file);
  if (found.present && found.text !== undefined) return { file, text: found.text };
  notes.push({ linter: "phpstan", text: `${file} is not read: ${NOT_A_FILE}` });
  return undefined;
}

export const phpstanReader: LinterReader = {
  linter: "phpstan",
  kinds: ["phpstan"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const used = usedConfig(root, notes);
    if (used === undefined) return undefined;
    const read: PhpstanRead = { layers: [], unread: [], registered: [] };
    if (!readText(root, used.file, used.text, 0, read, notes)) return undefined;
    for (const { rule, because } of read.registered) {
      notes.push({ linter: "phpstan", text: `${because} ${rule ?? "a rule"} registers a rule at every level, so the PHPStan rules Rerail ports that the level leaves off keep the pack's values unless it is the port's own rule.` });
    }
    const keep = "so the PHPStan rules Rerail ports keep the pack's values unless the config ignores them";
    for (const { what, why } of read.unread) notes.push({ linter: "phpstan", text: `${what} is not read (${why}), ${keep}.` });
    const level = levelOf(read);
    if ("unknown" in level) notes.push({ linter: "phpstan", text: `${used.file}: ${level.unknown}, ${keep}.` });
    return new PhpstanConfig(read);
  },
};
