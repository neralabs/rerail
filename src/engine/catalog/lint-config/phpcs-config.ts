/**
 * PHP_CodeSniffer's ruleset at the repository root, read as data (`xml-format.ts`) for the php pack's PHP_CodeSniffer
 * ports, as PHP_CodeSniffer 4.0.4 reads it (PHPCSStandards/PHP_CodeSniffer@bbdc3d0, the release
 * `packs/php/conformance.json` names).
 *
 * A message runs when its sniff is in the ruleset and its severity reaches the reporting threshold (5).
 * `Ruleset::processRuleset` (src/Ruleset.php l.528-890) builds the sniffs from `<rule ref>`s: a standard (whose
 * sniffs `phpcs-standards.generated.ts` lists), a category, a sniff, a message code, or another ruleset file, less
 * the sniffs its `<exclude>`s name (an excluded message code is severity 0 instead). `processRule` (l.1136-1370) sets
 * a rule's `<severity>` and `<exclude-pattern>`s on its ref and every sniff it newly includes; `File::addMessage`
 * (src/Files/File.php l.840-1030) takes the severity of the most specific of the message code, sniff, category and
 * standard that has one, and leaves out a file any of their patterns match. A sniff a standard includes is on by a
 * preset; one the ruleset names, explicitly. A pattern that is a folder (`src/Legacy/*`, or `tests` behind a leading
 * `*` and `/`) turns the rule off in that folder of the repository; any other pattern is a note. A standard or
 * ruleset Rerail does not read may include a sniff or set its severity: a port only presets decide keeps the pack's
 * value.
 */
import { posix } from "node:path";
import { LintConfigFormatError, readFormat } from "./formats/lint-config-formats.js";
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
import { PHPCS_STANDARDS, type PhpcsStandard } from "./generated/phpcs-standards.generated.js";
import { isRepositoryFolder, repositoryPath } from "./repository-paths.js";
import { parseXml, xmlChildren, type XmlElement } from "./formats/xml-format.js";

/** The config files PHP_CodeSniffer looks for, in its order (`Config::CONFIG_FILENAMES`, src/Config.php l.143-148); it uses the first present. */
const PHPCS_FILES = [".phpcs.xml", "phpcs.xml", ".phpcs.xml.dist", "phpcs.xml.dist"];
/** The severity a message needs to be reported by default (`errorSeverity`, `warningSeverity`; src/Config.php l.575-576). */
const REPORTING_THRESHOLD = 5;
/** The arguments that change the reporting threshold (src/Config.php `CLI_FLAGS_TO_SETTING_NAME`), and the `-n` flag. */
const THRESHOLD_ARGUMENTS = ["severity", "error-severity", "warning-severity"];
/** How many rulesets deep `<rule ref>`s to other ruleset files are followed. */
const MAX_RULESET_DEPTH = 8;
/** A folder pattern: path segments, written as PHP_CodeSniffer's own rulesets write them, with `*` only around them. */
const FOLDER_PATTERN = /^(\*?\/)?([A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_.-]+)*)(?:\/\**)?$/u;

/** A `<rule ref>` or `<exclude name>`, as PHP_CodeSniffer expands it (`Ruleset::expandRulesetReference`, l.955-1130). */
type Reference =
  | { readonly kind: "standard"; readonly name: string; readonly standard: PhpcsStandard }
  /** A category, sniff or message code, of any standard: what it names follows from the name. */
  | { readonly kind: "name"; readonly name: string }
  | { readonly kind: "ruleset"; readonly name: string; readonly ruleset: PhpcsRuleset }
  | { readonly kind: "unread"; readonly name: string; readonly why: string };

interface PhpcsRule {
  readonly ref: Reference;
  readonly excludes: readonly Reference[];
  readonly severity: number | undefined;
  readonly excludePatterns: readonly string[];
  readonly includePatterns: readonly string[];
  readonly properties: boolean;
}

interface PhpcsRuleset {
  readonly file: string;
  readonly rules: readonly PhpcsRule[];
  /** Its `<arg>`s by name (a flag argument by its value), the first of a name winning. */
  readonly args: ReadonlyMap<string, string>;
}

/** One message code (or a sniff, for a port that names no code) a port reports, and the codes its settings are looked up by. */
interface Leaf {
  readonly code: string;
  readonly sniff: string;
  /** The message code, sniff, category and standard, most specific first (`File::addMessage`'s `$checkCodes`). */
  readonly checkCodes: readonly string[];
}

/** A severity set on a code. */
interface Severity {
  readonly value: number;
  readonly because: string;
  readonly explicit: boolean;
}

/** What the rulesets set on one leaf, gathered as PHP_CodeSniffer processes them. */
interface LeafState {
  readonly severities: Map<string, Severity>;
  readonly excludePatterns: { readonly pattern: string; readonly because: string }[];
  readonly includePatterns: string[];
  readonly properties: string[];
  /** The references not read that may include the sniff or set its severity. */
  readonly unread: string[];
}

/** Where one ruleset leaves a sniff: included, by what, and excluded, by what. */
interface RulesetResult {
  readonly included: { readonly because: string; readonly explicit: boolean } | undefined;
  readonly excluded: string | undefined;
}

/** Whether PHP_CodeSniffer (not phpcbf) processes `element` (`Ruleset::shouldProcessElement`, l.1386-1410). */
function forPhpcs(element: XmlElement): boolean {
  const limited = element.attributes.has("phpcs-only") || element.attributes.has("phpcbf-only");
  return !limited || element.attributes.get("phpcs-only") === "true";
}

const processed = (parent: XmlElement, name: string): XmlElement[] => xmlChildren(parent, name).filter(forPhpcs);

/** Reads ruleset files, following the ones in the repository that rulesets reference. */
class RulesetReader {
  constructor(private readonly root: string) {}

  /** Ruleset file `file`; throws `LintConfigFormatError` when it is not XML Rerail reads, or not a ruleset. */
  ruleset(file: string, text: string, depth: number): PhpcsRuleset {
    const ruleset = parseXml(text);
    if (ruleset.localName !== "ruleset") throw new LintConfigFormatError("its root element is not <ruleset>");
    const rules = processed(ruleset, "rule").flatMap(rule => {
      const ref = rule.attributes.get("ref");
      return ref === undefined ? [] : [this.rule(file, rule, ref, depth)];
    });
    const args = new Map<string, string>();
    for (const arg of processed(ruleset, "arg")) {
      const name = arg.attributes.get("name") ?? `-${arg.attributes.get("value") ?? ""}`;
      if (!args.has(name)) args.set(name, arg.attributes.get("value") ?? "");
    }
    return { file, rules, args };
  }

  private rule(file: string, rule: XmlElement, ref: string, depth: number): PhpcsRule {
    const severity = processed(rule, "severity")[0]?.text.trim();
    return {
      ref: this.reference(file, ref, depth),
      excludes: processed(rule, "exclude").flatMap(exclude => {
        const name = exclude.attributes.get("name");
        return name === undefined ? [] : [this.reference(file, name, depth)];
      }),
      severity: severity === undefined ? undefined : Number.parseInt(severity, 10) || 0,
      excludePatterns: processed(rule, "exclude-pattern").map(pattern => pattern.text.trim()),
      includePatterns: processed(rule, "include-pattern").map(pattern => pattern.text.trim()),
      properties: processed(rule, "properties").length > 0,
    };
  }

  /** What reference `name` in ruleset `file` is. */
  private reference(file: string, name: string, depth: number): Reference {
    if (/^(?:\.|~\/)|[/\\]|\.(?:xml|php)$/u.test(name)) return this.pathReference(file, name, depth);
    const standard = PHPCS_STANDARDS[name];
    if (standard !== undefined) return { kind: "standard", name, standard };
    if (name.includes(".")) return { kind: "name", name };
    return { kind: "unread", name, why: "it is a coding standard PHP_CodeSniffer does not ship, which Rerail does not read" };
  }

  /** A reference to a ruleset file or folder, relative to the ruleset's folder when it starts with `.`, else to the root. */
  private pathReference(file: string, name: string, depth: number): Reference {
    const path = name.startsWith("~/") ? undefined : repositoryPath(name.startsWith(".") ? file.replace(/[^/]*$/u, "") : "", name);
    if (path === undefined) return { kind: "unread", name, why: "it is outside the repository" };
    const why = unreadRulesetPath(path, depth);
    if (why !== undefined) return { kind: "unread", name, why };
    return this.rulesetReference(name, isRepositoryFolder(this.root, path) ? posix.join(path, "ruleset.xml") : path, depth);
  }

  private rulesetReference(name: string, file: string, depth: number): Reference {
    const found = readRootFile(this.root, file);
    if (!found.present) return { kind: "unread", name, why: "it does not exist" };
    const text = found.text;
    if (text === undefined) return { kind: "unread", name, why: NOT_A_FILE.replace(/\.$/u, "") };
    const parsed = readFormat(() => this.ruleset(file, text, depth + 1));
    return parsed.ok ? { kind: "ruleset", name, ruleset: parsed.value } : { kind: "unread", name, why: parsed.why };
  }
}

/** Why the ruleset at `path` (relative to the root) is not read, if it is not. */
function unreadRulesetPath(path: string, depth: number): string | undefined {
  if (/(?:^|\/)vendor\//u.test(path)) return "it is an installed package's ruleset";
  if (path.endsWith(".php")) return "it is a sniff, which Rerail never runs";
  return depth >= MAX_RULESET_DEPTH ? `rulesets are followed ${MAX_RULESET_DEPTH} deep` : undefined;
}

/** The leaves of a port: its names no other of its names extends, as message codes or sniffs. */
function leavesOf(names: readonly string[]): Leaf[] {
  return names.filter(name => !names.some(other => other.startsWith(`${name}.`))).map(code => {
    const parts = code.split(".");
    const checkCodes = [4, 3, 2, 1].filter(length => length <= parts.length).map(length => parts.slice(0, length).join("."));
    return { code, sniff: parts.slice(0, 3).join("."), checkCodes: [...new Set(checkCodes)] };
  });
}

/** Whether name reference `name` (a category, sniff or message code) includes `sniff`. */
function namesSniff(name: string, sniff: string): boolean {
  const parts = name.split(".");
  return parts.length === 2 ? sniff.startsWith(`${name}.`) : parts.slice(0, 3).join(".") === sniff;
}

/** Replays the rulesets for one leaf, gathering into `state` what they set on it. */
class LeafReplay {
  readonly state: LeafState = { severities: new Map(), excludePatterns: [], includePatterns: [], properties: [], unread: [] };

  constructor(private readonly leaf: Leaf) {}

  ruleset(ruleset: PhpcsRuleset): RulesetResult {
    let included: RulesetResult["included"];
    let excluded: string | undefined;
    for (const rule of ruleset.rules) {
      const because = `${ruleset.file}: <rule ref="${rule.ref.name}">`;
      const includes = this.includes(rule.ref, because);
      const isNew = includes !== undefined && included === undefined;
      included ??= includes;
      if (rule.ref.kind === "name" && rule.ref.name.split(".").length === 4) this.singleMessage(rule.ref.name, because, isNew);
      excluded ??= this.excludes(ruleset.file, rule);
      this.processRule(rule, because, isNew);
    }
    return { included, excluded };
  }

  /** Whether `ref` includes the leaf's sniff, and how. */
  private includes(ref: Reference, because: string): RulesetResult["included"] {
    if (ref.kind === "unread") {
      this.state.unread.push(`${because} (${ref.why})`);
      return undefined;
    }
    if (ref.kind === "name") return namesSniff(ref.name, this.leaf.sniff) ? { because, explicit: true } : undefined;
    return ref.kind === "ruleset" ? this.nested(ref.ruleset, because) : this.standard(ref.name, ref.standard, because);
  }

  /** Whether built-in standard `name` includes the leaf's sniff (a preset), after its own severities, which processing it sets. */
  private standard(name: string, standard: PhpcsStandard, because: string): RulesetResult["included"] {
    for (const [code, value] of standard.severities) this.state.severities.set(code, { value, because: `${because}: ${name} sets severity ${value} for ${code}`, explicit: false });
    return standard.sniffs.includes(this.leaf.sniff) ? { because, explicit: false } : undefined;
  }

  /** Whether another ruleset includes the leaf's sniff, less the sniffs its own excludes leave out. */
  private nested(ruleset: PhpcsRuleset, because: string): RulesetResult["included"] {
    const result = this.ruleset(ruleset);
    return result.excluded === undefined && result.included !== undefined ? { because: `${because}: ${result.included.because}`, explicit: result.included.explicit } : undefined;
  }

  /**
   * Including one message code (l.719-748): back on if its sniff is at severity 0; else, when the rule newly includes
   * the sniff, only that message of it, the sniff's others at severity 0.
   */
  private singleMessage(code: string, because: string, isNew: boolean): void {
    const sniff = code.split(".").slice(0, 3).join(".");
    if (sniff !== this.leaf.sniff) return;
    if (this.state.severities.get(sniff)?.value !== 0) {
      if (!isNew) return;
      this.state.severities.set(sniff, { value: 0, because: `${because} includes only that message of ${sniff}`, explicit: true });
    }
    this.state.severities.set(code, { value: REPORTING_THRESHOLD, because, explicit: true });
  }

  /** A rule's `<exclude>`s: a message code's is severity 0; any other that names the leaf's sniff excludes it from the ruleset. */
  private excludes(file: string, rule: PhpcsRule): string | undefined {
    let excluded: string | undefined;
    for (const exclude of rule.excludes) {
      const because = `${file}: <rule ref="${rule.ref.name}"><exclude name="${exclude.name}"/>`;
      if (exclude.kind === "name" && exclude.name.split(".").length === 4) this.state.severities.set(exclude.name, { value: 0, because, explicit: true });
      else if (this.includes(exclude, because) !== undefined) excluded ??= because;
    }
    return excluded;
  }

  /** The leaf's codes `processRule` sets a rule's settings on: its ref, and for a standard, category or ruleset, each sniff it newly included. */
  private settingCodes(rule: PhpcsRule, isNew: boolean): string[] {
    const groups = rule.ref.kind !== "name" || rule.ref.name.split(".").length <= 2;
    return [rule.ref.name, ...groups && isNew ? [this.leaf.sniff] : []].filter(code => this.leaf.checkCodes.includes(code));
  }

  /** `processRule` (l.1136-1370): the rule's severity, patterns and properties, where they reach the leaf. */
  private processRule(rule: PhpcsRule, because: string, isNew: boolean): void {
    const codes = this.settingCodes(rule, isNew);
    if (codes.length === 0) return;
    const severity = rule.severity;
    if (severity !== undefined) for (const code of codes) this.state.severities.set(code, { value: severity, because: `${because}<severity>${severity}</severity>`, explicit: true });
    this.state.excludePatterns.push(...rule.excludePatterns.map(pattern => ({ pattern, because: `${because}<exclude-pattern>${pattern}</exclude-pattern>` })));
    this.state.includePatterns.push(...rule.includePatterns.map(pattern => `${because}<include-pattern>${pattern}</include-pattern>`));
    if (rule.properties) this.state.properties.push(because);
  }
}

/** The folder of the repository an exclude pattern leaves out, and whether it also matches files elsewhere; undefined for any other pattern. */
function patternFolder(root: string, pattern: string): { folder: string; elsewhere: boolean } | undefined {
  const match = FOLDER_PATTERN.exec(pattern);
  const written = match?.[2];
  const path = written === undefined ? undefined : repositoryPath("", written);
  if (path === undefined || path === "" || !isRepositoryFolder(root, path)) return undefined;
  return { folder: path, elsewhere: match?.[1] !== undefined };
}

/** What the rulesets decide for one leaf, for every file. */
type LeafDecision = PortDecision & { readonly code: string };

/** The read config: the ruleset and the arguments in force. */
interface PhpcsRead {
  readonly ruleset: PhpcsRuleset;
  /** The arguments, from the highest-level ruleset that sets each. */
  readonly args: ReadonlyMap<string, { readonly value: string; readonly because: string }>;
}

/** The arguments in force: a ruleset's own over those of the rulesets it references (l.623-700). */
function argsOf(ruleset: PhpcsRuleset, args = new Map<string, { value: string; because: string }>()): Map<string, { value: string; because: string }> {
  for (const [name, value] of ruleset.args) if (!args.has(name)) args.set(name, { value, because: `${ruleset.file}: <arg ${name.startsWith("-") ? "" : `name="${name}" `}value="${value}"/>` });
  for (const rule of ruleset.rules) if (rule.ref.kind === "ruleset") argsOf(rule.ref.ruleset, args);
  return args;
}

/** The argument that changes the reporting threshold, if any: a severity argument or the `-n` flag. */
function thresholdArgument(read: PhpcsRead): string | undefined {
  const named = THRESHOLD_ARGUMENTS.map(name => read.args.get(name)).find(arg => arg !== undefined);
  return (named ?? [...read.args].find(([name]) => name.startsWith("-") && name.includes("n"))?.[1])?.because;
}

/** The `--exclude` or `--sniffs` argument that leaves the leaf's sniff out, if any (`File::addMessage`, l.896-905). */
function argumentOff(read: PhpcsRead, leaf: Leaf): string | undefined {
  const sniff = leaf.sniff.toLowerCase();
  const lists = (arg: { value: string }): boolean => arg.value.toLowerCase().split(",").map(listed => listed.trim()).includes(sniff);
  const exclude = read.args.get("exclude");
  if (exclude !== undefined && lists(exclude)) return exclude.because;
  const sniffs = read.args.get("sniffs");
  return sniffs !== undefined && !lists(sniffs) ? sniffs.because : undefined;
}

/** What the rulesets decide for one leaf, from where its replay left it. */
function leafDecision(read: PhpcsRead, leaf: Leaf, result: RulesetResult, state: LeafState): LeafDecision {
  const code = leaf.code;
  const argument = argumentOff(read, leaf);
  if (argument !== undefined) return { code, on: false, explicit: true, because: argument };
  if (result.excluded !== undefined) return { code, on: false, explicit: true, because: result.excluded };
  const unread = state.unread[0];
  if (result.included === undefined) {
    return unread === undefined ? { code, on: false, explicit: false, because: `${read.ruleset.file} does not turn it on` } : { code, on: "unknown", because: `${unread} is not read and may include it` };
  }
  return severityDecision(leaf, result.included, state);
}

/** The decision for an included leaf: off when its severity is below the threshold, else on. */
function severityDecision(leaf: Leaf, included: NonNullable<RulesetResult["included"]>, state: LeafState): LeafDecision {
  const severity = leaf.checkCodes.map(code => state.severities.get(code)).find(found => found !== undefined);
  if (severity !== undefined && severity.value < REPORTING_THRESHOLD) return { code: leaf.code, on: false, explicit: severity.explicit, because: severity.because };
  const explicit = included.explicit || severity?.explicit === true;
  const unread = state.unread[0];
  if (!explicit && unread !== undefined) return { code: leaf.code, on: "unknown", because: `${unread} is not read and may set its severity` };
  return { code: leaf.code, on: true, explicit, because: included.because };
}

/** The decisions of a port's leaves as one, noting the leaves a mixed decision leaves out. */
function combined(port: Port, decisions: readonly LeafDecision[], notes: string[]): PortDecision {
  const unknown = decisions.find(decision => decision.on === "unknown");
  if (unknown !== undefined) return { on: "unknown", because: unknown.because };
  const on = decisions.filter(decision => decision.on === true);
  const deciding = on.length > 0 ? on : decisions;
  if (on.length > 0 && on.length < decisions.length) {
    const off = decisions.filter(decision => decision.on === false);
    notes.push(`${port.ruleId}: ${[...new Set(off.map(decision => decision.because))].join("; ")} turns off ${off.map(decision => decision.code).join(", ")}; the rule stays on for its other messages.`);
  }
  const explicit = deciding.some(decision => decision.on !== "unknown" && decision.explicit);
  return { on: on.length > 0, explicit, because: [...new Set(deciding.map(decision => decision.because))].join("; ") };
}

/** The folders every leaf's exclude patterns leave out, noting the patterns that are not folders or leave out only some leaves. */
function folderDecisions(root: string, port: Port, states: readonly LeafState[], notes: string[]): FolderDecision[] {
  const patterns = new Map(states.flatMap(state => state.excludePatterns).map(entry => [entry.pattern, entry.because]));
  return [...patterns].flatMap(([pattern, because]): FolderDecision[] => {
    const folder = patternFolder(root, pattern);
    const everyLeaf = states.every(state => state.excludePatterns.some(entry => entry.pattern === pattern));
    if (folder === undefined || !everyLeaf) {
      notes.push(`${port.ruleId}: ${because} leaves out ${folder === undefined ? "files that are not one folder of the repository" : "only some of its messages"}; Rerail sets rules for whole folders only.`);
      return [];
    }
    if (folder.elsewhere) notes.push(`${port.ruleId}: ${because} also leaves out folders named ${folder.folder} below the root; the rule stays on there.`);
    return [{ folder: folder.folder, on: false, because }];
  });
}

/** Notes what the rulesets set on a port that Rerail does not use: include patterns, properties. */
function noteUnused(port: Port, states: readonly LeafState[], notes: string[]): void {
  for (const because of new Set(states.flatMap(state => state.includePatterns))) notes.push(`${port.ruleId}: ${because} limits it to some files; it is not limited.`);
  for (const because of new Set(states.flatMap(state => state.properties))) notes.push(`${port.ruleId}: the properties of ${because} are not mapped; it keeps its default options.`);
}

class PhpcsConfig implements LinterConfig {
  constructor(private readonly root: string, private readonly read: PhpcsRead) {}

  verdict(port: Port): PortVerdict | undefined {
    if (port.suppress.phpcs.length === 0) return undefined;
    const notes: string[] = [];
    const replays = leavesOf(port.suppress.phpcs).map(leaf => {
      const replay = new LeafReplay(leaf);
      return { leaf, result: replay.ruleset(this.read.ruleset), state: replay.state };
    });
    const states = replays.map(replay => replay.state);
    const decision = this.#thresholdDecision(combined(port, replays.map(({ leaf, result, state }) => leafDecision(this.read, leaf, result, state)), notes));
    const folders = folderDecisions(this.root, port, states, notes);
    noteUnused(port, states, notes);
    return { decision, ...folders.length > 0 ? { folders } : {}, ...notes.length > 0 ? { notes } : {} };
  }

  /** A reporting threshold Rerail does not read leaves the pack's value, unless the rule is turned off by name. */
  #thresholdDecision(decision: PortDecision): PortDecision {
    const threshold = thresholdArgument(this.read);
    if (threshold === undefined || (decision.on === false && decision.explicit)) return decision;
    return { on: "unknown", because: `${threshold} changes which severities are reported` };
  }
}

/** The ruleset PHP_CodeSniffer uses at the root, with its text; undefined (with a note when present but not read) otherwise. */
function usedConfig(root: string, notes: LinterNote[]): { file: string; text: string } | undefined {
  const present = PHPCS_FILES.filter(name => readRootFile(root, name).present);
  const file = present[0];
  if (file === undefined) return undefined;
  for (const name of present.slice(1)) notes.push({ linter: "phpcs", text: `${name} is not read: PHP_CodeSniffer uses ${file} instead.` });
  const found = readRootFile(root, file);
  if (found.present && found.text !== undefined) return { file, text: found.text };
  notes.push({ linter: "phpcs", text: `${file} is not read: ${NOT_A_FILE}` });
  return undefined;
}

/** Notes each reference not read, once, wherever it is. */
function noteUnread(ruleset: PhpcsRuleset, notes: LinterNote[]): void {
  for (const rule of ruleset.rules) {
    for (const ref of [rule.ref, ...rule.excludes]) {
      if (ref.kind === "unread") notes.push({ linter: "phpcs", text: `${ruleset.file}: ${JSON.stringify(ref.name)} is not read (${ref.why}), so the PHP_CodeSniffer rules Rerail ports that only a standard turns on keep the pack's values.` });
      if (ref.kind === "ruleset") noteUnread(ref.ruleset, notes);
    }
  }
}

export const phpcsReader: LinterReader = {
  linter: "phpcs",
  kinds: ["phpcs"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const used = usedConfig(root, notes);
    if (used === undefined) return undefined;
    const parsed = readFormat(() => new RulesetReader(root).ruleset(used.file, used.text, 0));
    if (!parsed.ok) {
      notes.push({ linter: "phpcs", text: `${used.file} is not read: ${parsed.why}.` });
      return undefined;
    }
    const ruleset = parsed.value;
    noteUnread(ruleset, notes);
    const read: PhpcsRead = { ruleset, args: argsOf(ruleset) };
    const threshold = thresholdArgument(read);
    if (threshold !== undefined) notes.push({ linter: "phpcs", text: `${threshold} changes which severities are reported, so the PHP_CodeSniffer rules Rerail ports keep the pack's values unless the config turns them off by name.` });
    return new PhpcsConfig(root, read);
  },
};
