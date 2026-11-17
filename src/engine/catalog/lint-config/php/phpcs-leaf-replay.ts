/**
 * The rulesets replayed for each message code a PHP_CodeSniffer port reports, as PHP_CodeSniffer processes them, and
 * what they decide for it. A message runs when its sniff is in the ruleset and its severity reaches the reporting
 * threshold (5). A rule includes the sniffs its ref names less the sniffs its `<exclude>`s name (an excluded message
 * code is severity 0 instead). `processRule` (src/Ruleset.php l.1136-1370) sets a rule's `<severity>` and patterns on
 * its ref and, for a ref of at most two dot-separated parts or with an empty one, every sniff it newly includes
 * (`phpcs.xml.dist`'s reach no sniff); `File::addMessage` (src/Files/File.php l.840-1030) takes the severity of the most
 * specific of the message code, sniff, category and standard that has one. A sniff a standard includes is on by a
 * preset; one the ruleset names, explicitly. A standard or ruleset Rerail does not read may include a sniff or set its
 * severity: a port only presets decide keeps the pack's value.
 */
import type { Port, PortDecision } from "../linter-config.js";
import type { PhpcsStandard } from "../generated/phpcs-standards.generated.js";
import { argumentOff, type PhpcsRead } from "./phpcs-arguments.js";
import type { Pattern, PhpcsRule, PhpcsRuleset, Reference } from "./phpcs-ruleset.js";

/** The severity a message needs to be reported by default (`errorSeverity`, `warningSeverity`; src/Config.php l.575-576). */
const REPORTING_THRESHOLD = 5;

/** One message code (or a sniff, for a port that names no code) a port reports, and the codes its settings are looked up by. */
export interface Leaf {
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
  readonly excludePatterns: Pattern[];
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

/** The leaves of a port: its names no other of its names extends, as message codes or sniffs. */
export function leavesOf(names: readonly string[]): Leaf[] {
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
export class LeafReplay {
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

  /**
   * The leaf's codes `processRule` sets a rule's settings on: its ref, and each sniff it newly included when the ref
   * has at most two dot-separated parts or an empty one (a standard, a category, `./base.xml`; l.1141-1161), as
   * PHP_CodeSniffer's `array_filter` reads parts: `0` is empty too. A ruleset named `phpcs.xml.dist` passes none on.
   */
  private settingCodes(rule: PhpcsRule, isNew: boolean): string[] {
    const parts = rule.ref.name.split(".");
    const groups = parts.length <= 2 || parts.some(part => part === "" || part === "0");
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

/** What the rulesets decide for one leaf, for every file. */
type LeafDecision = PortDecision & { readonly code: string };

/** What the rulesets decide for one leaf, from where its replay left it. */
export function leafDecision(read: PhpcsRead, leaf: Leaf, result: RulesetResult, state: LeafState): LeafDecision {
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
export function combined(port: Port, decisions: readonly LeafDecision[], notes: string[]): PortDecision {
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
