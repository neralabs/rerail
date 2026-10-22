import type { CorpusFile } from "./corpus.js";
import type { GatesManifest } from "./manifest.js";

export type { EslintUpstream, KnownGap, RealCodeGap } from "./manifest.js";

/**
 * The gates in the order a rule meets them: the `conformance` gates first, which hold for every rule document of every
 * pack (conformance-gates.ts judges the rules no gated tool-pack does), then the `port` gates, which judge a port
 * against the upstream linter it ports, so only a gated tool-pack's rules meet them. `loads` to
 * `matches-upstream-tests` decide whether a rule ships (`blocking`); `upstream-status` only decides whether it ships on
 * or off. `passes-conformance` is the pack contract's own check (`test:conformance`), so the gates are a superset of
 * it. V1's gates ran compile, generality, size, upstream tests; here size (`compact`) and the pack's check come before
 * generality (`not-overfit`), so a rule's first failure is a conformance one whenever it has one. V1's Gate 6 (real
 * code) is NRV's alone: rerail judges the goldens it imports from NRV, in `matches-upstream-tests`.
 */
export const GATES = [
  { id: "upstream-status", title: "Upstream status (decides on or off)", blocking: false, section: "port" },
  { id: "loads", title: "Compiles and loads on this engine", blocking: true, section: "conformance" },
  { id: "compact", title: "Compact (no written-out expansion)", blocking: true, section: "conformance" },
  { id: "passes-conformance", title: "Passes the pack's conformance check", blocking: true, section: "conformance" },
  { id: "not-overfit", title: "Not overfit to upstream's tests", blocking: true, section: "port" },
  { id: "matches-upstream-tests", title: "Matches upstream on its own tests", blocking: true, section: "port" },
] as const;

type BlockingGate = Extract<(typeof GATES)[number], { blocking: true }>;

/** The gates a rule must pass to ship, in the order a verdict reads them. */
export const BLOCKING_GATES = GATES.filter((gate): gate is BlockingGate => gate.blocking);

export type GateId = (typeof GATES)[number]["id"];
export type BlockingGateId = BlockingGate["id"];
export type GateSection = (typeof GATES)[number]["section"];

type ConformanceGate = Extract<BlockingGate, { section: "conformance" }>;
type PortGate = Extract<BlockingGate, { section: "port" }>;

/** The blocking gates every rule meets, ported or not: `loads`, `compact`, `passes-conformance`. */
export const CONFORMANCE_GATES = BLOCKING_GATES.filter((gate): gate is ConformanceGate => gate.section === "conformance");
/** The blocking gates only a gated tool-pack's rules meet: `not-overfit`, `matches-upstream-tests`. */
export const PORT_GATES = BLOCKING_GATES.filter((gate): gate is PortGate => gate.section === "port");

export type ConformanceGateId = ConformanceGate["id"];
export type PortGateId = PortGate["id"];

/** One deterministic check. `detail` says what was compared, so a failure explains itself. */
/**
 * One fixtureTexts() entry's origin: a file of upstream's checkout (`path` relative to it, `sha256` of the text as the
 * adapter produced it, `transform` when that text is not the raw file), or an inline text that is no file.
 */
export type FixtureTextSource =
  | { path: string; sha256: string; transform?: "notebook-code-cells" }
  | { inline: string };

export interface Check {
  id: string;
  ok: boolean;
  detail: string;
}

/** The verdict of the blocking gates: the first one the rule fails, or null when it passes them all and ships. */
export function verdict(gates: Record<BlockingGateId, { pass: boolean }>): { pass: boolean; stoppedAt: BlockingGateId | null } {
  const stopped = BLOCKING_GATES.find((entry) => !gates[entry.id].pass);
  return { pass: stopped === undefined, stoppedAt: stopped?.id ?? null };
}

/** A tool-pack's `gates.yaml`, with the id the gates name the tool-pack by: `<pack>/<tool>` (`python/ruff`). */
export interface PackManifest extends GatesManifest {
  id: string;
}

/** One rule document of a pack, read as raw bytes. */
export interface PackRule {
  /** The file name, e.g. `no-debugger.json`. */
  file: string;
  /** Repository-relative path, under the pack's `rules/` or `candidates/`. */
  path: string;
  /** The upstream rule the document ports, as the pack's conformance.json registers it. */
  rule: string;
  text: string;
  bytes: number;
  sha256: string;
  /** The document's `id`, when the bytes are a JSON object with a string id; `loads` judges the rest. */
  id: string | null;
}

/**
 * A finding's position: 0-based lines and columns, with columns in the unit upstream counts in (ESLint UTF-16 code
 * units, Ruff code points, Bandit UTF-8 bytes), and only the parts upstream reports. A part upstream does not report
 * is null on both sides of a comparison, so `matches-upstream-tests` compares exactly the positions upstream reports.
 */
export interface Span {
  startLine: number;
  startCol: number | null;
  endLine: number | null;
  endCol: number | null;
}

/**
 * The end parts upstream reports for one rule; an engine span keeps only these (span-units/), and always its start
 * line and column. Ruff reports both ends; Bandit an end column without an end line, and no end for B613; ESLint both,
 * or neither for a rule some of whose findings on its own tests lack them.
 */
export interface SpanParts {
  endLine: boolean;
  endCol: boolean;
}

/** What the upstream implementation says about one rule at the pinned version. */
export interface UpstreamRule {
  name: string;
  implementation: string[];
  tests: string[];
  /** The deprecation message upstream's own metadata reports at the pinned runtime, or null. */
  deprecation: string | null;
  /** The first approved configuration that turns the rule on, with the options it sets; null when none does. */
  recommended: { config: string; options: unknown[] } | null;
  /** Why upstream cannot lint real code for the rule (a type-checked rule), or null when it can: its corpus answers are empty. */
  notOnRealCode: string | null;
}

/**
 * One independent parser's verdict on a source file: `syntax` errors stop it building a tree; `grammar` errors are
 * what the language forbids in a tree it did build (a compiler's grammar checks, a parser's recoverable early errors).
 * `grammar` is null for a parser that makes no grammar checks at all, such as libcst.
 */
export interface ParserVerdict {
  parser: string;
  syntax: string[];
  grammar: string[] | null;
}

/** One upstream test file, linted by upstream with the configuration the shipped document implements. */
export interface UpstreamCase {
  /** Upstream-relative fixture path, e.g. `test/rules/quotemark/single/test.ts.lint`. */
  fixture: string;
  /** The upstream test configuration that owns the file, relative to the rule's test directory (`.` for the rule directory). */
  variant: string;
  /** The path the engine evaluates; the upstream file name without its fixture extension. */
  path: string;
  /**
   * The file type of the language upstream's parser read the case in, when the path's file type is another language's
   * (JSX at `pages/index.ts`): the engine reads the case in that language at `path`.
   */
  parsedAs?: string;
  source: string;
  /**
   * ECMAScript code upstream reads as a script rather than a module (ESLint's `sourceType: "script"`), which the file
   * type does not say; it decides what the independent parsers read as strict-mode code.
   */
  script?: boolean;
  /** Why upstream itself did not lint this file under this configuration, or null when it did. */
  skipped: string | null;
  /** The upstream linter's output on this file, configured as upstream recommends: the expected answer. */
  expected: Span[];
  /**
   * Upstream's written expectation, when the file's own test configuration is the shipped one; null otherwise. Only
   * used to report when upstream's output disagrees with it.
   */
  markup: Span[] | null;
}

/** One corpus file with upstream's verdict for every rule of the pack, computed once per run. */
export interface UpstreamFile {
  file: CorpusFile;
  /** Why upstream gives no ground truth for the file (its own parser rejects it), or null. */
  excluded: string | null;
  findings: Map<string, Span[]>;
}

/** Everything the gates need from one upstream linter: a live adapter (recorders/port-readiness/adapters/) or its recording. */
export interface PackAdapter {
  /** Upstream identity recorded in the report. */
  identity: Record<string, string>;
  /** Tool-pack checks only the adapter can make: pins, runtime identity, declared license. */
  packChecks: Check[];
  /** The upstream LICENSE text at the pinned commit. */
  licenseText: string;
  rule(name: string): UpstreamRule | undefined;
  /** Names upstream's own implementation of the rule checks. */
  implementationNames(name: string): Set<string>;
  /** Names checked by the upstream helpers the rule delegates to (a shared utility module the rule imports). */
  sharedImplementationNames(name: string): Set<string>;
  /** The regexes upstream's implementation of the rule and its helpers match names with. */
  implementationRegexes(name: string): RegExp[];
  /** Every word in upstream's test files for all its rules, and in the option values of their test configurations. */
  fixtureWords(): Set<string>;
  /** The text of every upstream test file, as the linter reads it. */
  fixtureTexts(): string[];
  /**
   * Where each fixtureTexts() entry comes from, in the same order (live adapters only): an upstream file at the pinned
   * commit, or a text that is no file. The recorder writes it so the gates can rebuild the same list.
   */
  fixtureTextSources?(): FixtureTextSource[];
  /** Keywords of the upstream language: syntax wherever a rule spells them. */
  keywords: Set<string>;
  /** Words the upstream language gives meaning to inside comments (documentation tags, directives). */
  commentVocabulary: Set<string>;
  /** Paths a ported rule must be in scope for, one per file type upstream lints. */
  inScopeSamples: string[];
  /** Upstream's output on its own test files for the rule, configured as upstream recommends. */
  cases(name: string): UpstreamCase[];
  /** Convert an engine byte span to upstream's coordinates and to the precision upstream reports for the rule. */
  toSpan(name: string, source: string, startByte: number, endByte: number): Span;
  /** Whether a corpus file is of a type upstream lints, so a golden can name it. */
  inCorpus(path: string): boolean;
  /**
   * Upstream's findings on each corpus file for every named rule, configured as upstream recommends; a file
   * upstream's own parser rejects is excluded, with the reason.
   */
  lintCorpus(files: CorpusFile[], names: string[]): Promise<UpstreamFile[]>;
  /** Independent parsers' verdicts on each file, to tell invalid source from a gap in the engine's parser. */
  parserVerdicts(files: { path: string; source: string; script?: boolean }[]): Promise<ParserVerdict[][]>;
}
