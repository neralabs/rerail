import { extname } from "node:path";

import type { CompiledPolicy, Engine, EngineEvaluation } from "../engine.js";
import { formatSpan, spanKey, startKey, unmatched } from "../spans.js";
import type { CorpusFile } from "../corpus.js";
import type { Golden } from "../goldens.js";
import type { Check, KnownGap, PackAdapter, ParserVerdict, Span, UpstreamCase } from "../types.js";

/** How matches-upstream-tests judges, recorded in every report so a reader knows what "identical" and "excused" meant on that run. */
export const MATCHES_UPSTREAM_TESTS_METHOD = {
  expected: "the upstream source linter's own output on each of its own test files, configured as upstream recommends (the rule on for the languages the document covers, with the options of the first approved configuration that turns it on)",
  unreachable: "upstream findings in a file the document cannot apply to (outside its scope, or a file type the engine has no dialect for) are differences",
  identical: "the same findings in both directions (nothing missing, nothing extra), compared on every position upstream reports (start and end, in upstream's coordinates); messages are not compared",
  evaluation: "stateless CompiledCodeRuleDsl.evaluate in report mode, one file per snapshot, with no wall-clock budget; production evaluates through the workspace path with a 500 ms per-file budget",
  excused: "a differing file is excused only when a listed known gap that still reproduces names it and the engine reported a parse failure on it; nothing else excuses a difference",
  fires: "at least one upstream finding reproduced in an upstream test file that agrees: the golden test that proves a shipped rule fires, so a rule without one does not ship",
  "stays silent": "at least one case where upstream reports nothing and the rule agrees, from an upstream test file, a golden, or one of matches-real-code's negative files: the golden test that proves a shipped rule stays silent, so a rule without one does not ship",
  goldens: "the rule produces exactly what the pinned upstream reports on every golden of the tool-pack's goldens.tsv (each a real-code file where matches-real-code once found the rule and upstream differ, read from the pinned corpus and checked against its sha256), compared as upstream's test files are; a golden whose corpus file is gone from the pinned corpus fails",
  "engine gaps": "a golden that differs where the engine could not fully read the file (it reported source-partial, source-incomplete or budget-exceeded) is an engine gap: identical-on-goldens judges only the goldens the engine read fully, and engine-reads-goldens fails the rule on every engine gap, since the rule must not ship until the engine reads the file; the fix belongs in the engine (NER-1764), and once the engine reads the file the golden is judged like any other",
  "gap candidates": "every file the engine cannot fully parse is read by independent parsers, each reporting syntax errors and grammar errors (what the language forbids in a tree the parser built): invalid source when every parser reports one, valid source (the engine's parser is wrong) when none does, and parsers disagree otherwise",
} as const;

function gapNote(results: CaseResult[]): string {
  const excused = results.filter((result) => result.status === "excused").length;
  return excused === 0 ? "" : `; ${excused} file(s) excused by listed known gaps`;
}

/**
 * The path whose file type gives the language the engine reads a case in: its own, or, where upstream's parser read
 * another language than the path's (`parsedAs`), the path with that file type. The case is still evaluated at its path.
 */
export function readPath(testCase: Pick<UpstreamCase, "path" | "parsedAs">): string {
  return testCase.parsedAs === undefined ? testCase.path : testCase.path.replace(/\.[^./]*$/u, testCase.parsedAs);
}

/** Engine failure codes that mean the file did not fully parse. */
export const PARSE_FAILURE_CODES = new Set(["source-partial", "source-incomplete"]);

/** Engine failure codes that mean the engine did not fully read the file: a parse failure, or its work budget exceeded. */
export const ENGINE_GAP_CODES = new Set([...PARSE_FAILURE_CODES, "budget-exceeded"]);

export type CaseStatus = "agree" | "mismatch" | "excused" | "not-scored";

/**
 * A file the engine could not fully parse (an upstream test file no listed gap covers, or a corpus file), with
 * independent parsers' verdicts. `invalid-source`: every parser reports a syntax or grammar error, so the file is not
 * valid source and the failure is the file's. `valid-source`: no parser reports any, so the engine's parser is wrong.
 * `parsers-disagree`: some do and some do not.
 */
export interface GapCandidate {
  gate: "matches-upstream-tests" | "matches-real-code";
  /** The upstream test file, or `<task>/<path>` for a corpus file. */
  file: string;
  engine: string;
  verdict: "invalid-source" | "valid-source" | "parsers-disagree";
  parsers: ParserVerdict[];
}

export function classifyParsers(parsers: ParserVerdict[]): GapCandidate["verdict"] {
  const rejects = (verdict: ParserVerdict) => verdict.syntax.length + (verdict.grammar?.length ?? 0) > 0;
  // When every reported error is a grammar error, a parser that makes no grammar checks has no say.
  const grammarOnly = parsers.every((verdict) => verdict.syntax.length === 0);
  const voters = parsers.filter((verdict) => rejects(verdict) || !(grammarOnly && verdict.grammar === null));
  const rejecting = voters.filter(rejects).length;
  if (rejecting === 0) return "valid-source";
  return rejecting === voters.length ? "invalid-source" : "parsers-disagree";
}

export interface CaseResult {
  fixture: string;
  variant: string;
  path: string;
  status: CaseStatus;
  reason: string;
  expected: number;
  actual: number;
  /** Upstream findings the rule did not produce, and findings upstream did not produce. */
  missing: Span[];
  extra: Span[];
  /** True when the findings agree on start positions, whatever their ends. */
  startsAgree: boolean;
  failures: string[];
}

export interface MatchesUpstreamTestsResult {
  checks: Check[];
  cases: CaseResult[];
  totals: { scoredCases: number; expected: number; matched: number; missing: number; extra: number; startsMatched: number };
  /** Excuses each known gap granted, by gap id. */
  gapUses: Record<string, number>;
  /** Upstream's own test expectations that disagree with upstream's output under the pinned runtime. */
  markupDisagreements: string[];
  /** Upstream test files the engine could not fully parse that no listed gap covers; run.ts asks independent parsers. */
  unparsed: UnparsedFile[];
}

/** A file the engine could not fully parse, before independent parsers read it. */
export interface UnparsedFile {
  gate: GapCandidate["gate"];
  file: string;
  path: string;
  source: string;
  /** Read as an ECMAScript script rather than a module, as upstream read it. */
  script: boolean;
  engine: string;
}


/** The root node kind of each policy language's grammar, which a parse probe matches. */
const ROOT_KIND: Record<string, { language: string; kind: string }> = {
  python: { language: "python", kind: "module" },
  typescript: { language: "typescript", kind: "program" },
  tsx: { language: "typescript", kind: "program" },
  javascript: { language: "javascript", kind: "program" },
};

/** Whether the engine fails to fully parse a probe, using a policy that matches the root of every file. */
export function engineRejects(engine: Engine, path: string, source: string): boolean {
  const dialect = engine.rerail.dialectForPath(path);
  const root = dialect === undefined ? undefined : ROOT_KIND[dialect];
  if (dialect === undefined || root === undefined) throw new Error(`no parse probe for ${path}: the engine has no dialect for it, or the probe no root kind`);
  const probe = engine.compile(JSON.stringify({
    format: "rerail.policy/v1",
    id: "validation:parse-probe",
    revision: 1,
    mode: "log",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{
      id: "program",
      forbid: { ast: { language: root.language, rule: { kind: root.kind } } },
      violation: { anchor: "file.start", code: "PROBE", message: "probe" },
    }],
  }));
  return probe.evaluateFile(path, dialect, source).failures.some((failure) => PARSE_FAILURE_CODES.has(failure.code));
}

/** A listed gap this run proved again: the engine still fails to parse its probe, as the file type its fixtures share. */
export interface ProvenGap {
  gap: KnownGap;
  /** The path the probe was parsed as; run.ts asks independent parsers about the probe under it. */
  probePath: string;
}

/**
 * Re-prove every listed gap on this engine: the engine must still fail to parse its probe, under the file type
 * of the upstream test files it names (`casePaths` maps each upstream test file to the path the engine evaluates). A
 * gap that no longer reproduces excuses nothing.
 */
export function provenGaps(engine: Engine, gaps: KnownGap[], casePaths: Map<string, string>): { proven: Map<string, ProvenGap>; problems: string[] } {
  const proven = new Map<string, ProvenGap>();
  const problems: string[] = [];
  for (const gap of gaps) {
    const unknown = gap.fixtures.filter((fixture) => !casePaths.has(fixture));
    if (unknown.length > 0) {
      problems.push(`known gap ${gap.id}: ${unknown[0]} is not an upstream test file of the pack`);
      continue;
    }
    // A gap is proven under the file type of the fixtures it names: a JSX probe means nothing parsed as plain TypeScript.
    const extensions = [...new Set(gap.fixtures.map((fixture) => extname(casePaths.get(fixture)!)))];
    if (extensions.length !== 1 || extensions[0] === "") {
      problems.push(`known gap ${gap.id}: its fixtures must share one file type, found ${extensions.join(", ") || "none"}`);
      continue;
    }
    const probePath = `probe${extensions[0]}`;
    if (!engineRejects(engine, probePath, gap.probe)) {
      problems.push(`known gap ${gap.id}: the engine parses the probe as ${extensions[0]}, so the gap is closed`);
      continue;
    }
    proven.set(gap.id, { gap, probePath });
  }
  return { proven, problems };
}

/**
 * matches-upstream-tests: run the rule on every upstream test file upstream lints under the shipped configuration, and
 * require the same findings as upstream's own output, span for span (start and end, in upstream's coordinates). A
 * mismatch is excused only when a proven known gap lists the file and the engine reported a parse failure on it, as the
 * spec's "apart from known parser gaps we have listed" requires. The rule also needs at least one upstream finding in
 * agreeing files.
 */
export function matchesUpstreamTestsGate(
  engine: Engine,
  adapter: PackAdapter,
  name: string,
  compiled: CompiledPolicy,
  cases: UpstreamCase[],
  gaps: Map<string, ProvenGap>,
): MatchesUpstreamTestsResult {
  const results: CaseResult[] = [];
  const gapUses: Record<string, number> = {};
  const markupDisagreements: string[] = [];
  const unparsed: UnparsedFile[] = [];
  const totals = { scoredCases: 0, expected: 0, matched: 0, missing: 0, extra: 0, startsMatched: 0 };
  let evidence = 0;

  for (const testCase of cases) {
    const base = { fixture: testCase.fixture, variant: testCase.variant, path: testCase.path, expected: testCase.expected.length };
    const notScored = (reason: string): CaseResult => ({ ...base, status: "not-scored", reason, actual: 0, missing: [], extra: [], startsAgree: true, failures: [] });
    if (testCase.skipped !== null) {
      results.push(notScored(testCase.skipped));
      continue;
    }
    if (testCase.markup !== null
      && (unmatched(testCase.expected, testCase.markup, spanKey).length > 0 || unmatched(testCase.markup, testCase.expected, spanKey).length > 0)) {
      markupDisagreements.push(`${testCase.fixture}: upstream's output differs from its own test markup`);
    }
    const language = engine.rerail.dialectForPath(readPath(testCase));
    const unreachable = language === undefined
      ? `RERAIL has no dialect for ${extname(testCase.path) || "files without an extension"}`
      : !compiled.inScope(testCase.path)
        ? "the file is outside the shipped document's scope"
        : undefined;
    if (unreachable !== undefined || language === undefined) {
      if (testCase.expected.length === 0) {
        results.push(notScored(unreachable ?? "no dialect"));
        continue;
      }
      // Upstream reports findings the rule can never produce here: a difference, not a file to skip.
      totals.scoredCases += 1;
      totals.expected += testCase.expected.length;
      totals.missing += testCase.expected.length;
      results.push({ ...base, status: "mismatch", reason: `${unreachable ?? "no dialect"}, but upstream reports ${testCase.expected.length} finding(s) in it`, actual: 0, missing: testCase.expected, extra: [], startsAgree: false, failures: [] });
      continue;
    }

    const evaluation: EngineEvaluation = compiled.evaluateFile(testCase.path, language, testCase.source);
    const actual = evaluation.violations.map((violation) => adapter.toSpan(name, testCase.source, violation.span.startByte, violation.span.endByte));
    const missing = unmatched(testCase.expected, actual);
    const extra = unmatched(actual, testCase.expected);
    const startsAgree = unmatched(testCase.expected, actual, startKey).length === 0 && unmatched(actual, testCase.expected, startKey).length === 0;
    const parseFailure = evaluation.failures.some((failure) => failure.code === "source-partial" || failure.code === "source-incomplete");
    const blocking = evaluation.failures.filter((failure) => failure.code !== "source-partial");
    const failures = evaluation.failures.map((failure) => `${failure.code}: ${failure.message}`);
    if (parseFailure && ![...gaps.values()].some(({ gap }) => gap.fixtures.includes(testCase.fixture))) {
      unparsed.push({ gate: "matches-upstream-tests", file: testCase.fixture, path: readPath(testCase), source: testCase.source, script: testCase.script === true, engine: failures.find((failure) => /^source-/u.test(failure)) ?? "" });
    }

    totals.scoredCases += 1;
    totals.expected += testCase.expected.length;
    totals.matched += testCase.expected.length - missing.length;
    totals.missing += missing.length;
    totals.extra += extra.length;
    totals.startsMatched += testCase.expected.length - unmatched(testCase.expected, actual, startKey).length;

    const result = { ...base, actual: actual.length, missing, extra, startsAgree, failures };
    if (missing.length === 0 && extra.length === 0 && blocking.length === 0) {
      evidence += testCase.expected.length;
      results.push({ ...result, status: "agree", reason: "identical findings" });
      continue;
    }
    const difference = [
      ...missing.slice(0, 3).map((span) => `missing ${formatSpan(span)}`),
      ...extra.slice(0, 3).map((span) => `extra ${formatSpan(span)}`),
      ...blocking.slice(0, 1).map((failure) => `engine ${failure.code}`),
    ].join(", ");
    const gap = [...gaps.values()].map(({ gap: listed }) => listed).find((candidate) => candidate.fixtures.includes(testCase.fixture));
    if (gap !== undefined && parseFailure) {
      gapUses[gap.id] = (gapUses[gap.id] ?? 0) + 1;
      results.push({ ...result, status: "excused", reason: `known parser gap ${gap.id} (${difference})` });
      continue;
    }
    results.push({ ...result, status: "mismatch", reason: difference });
  }

  const mismatches = results.filter((result) => result.status === "mismatch");
  return {
    checks: [
      {
        id: "identical-results",
        ok: mismatches.length === 0,
        detail: mismatches.length === 0
          ? `identical on ${totals.scoredCases - results.filter((result) => result.status === "excused").length} scored file(s)${gapNote(results)}`
          : `${mismatches.length} upstream test file(s) differ, e.g. ${mismatches[0]!.fixture}: ${mismatches[0]!.reason}`,
      },
      {
        id: "fires",
        ok: evidence > 0,
        detail: `${evidence} upstream finding(s) reproduced in agreeing files`,
      },
    ],
    cases: results,
    totals,
    gapUses,
    markupDisagreements,
    unparsed,
  };
}

/** One golden of the rule, judged by matches-upstream-tests. */
export interface GoldenResult {
  task: string;
  path: string;
  status: "agree" | "mismatch" | "engine-gap";
  reason: string;
  expected: number;
  actual: number;
  missing: Span[];
  extra: Span[];
  failures: string[];
}

/**
 * matches-upstream-tests's identical-on-goldens check: on every golden, the rule must report exactly what the pinned
 * upstream reports there, in both directions, with no blocking engine failure, as on upstream's own test files. `file` is the golden's
 * corpus file in the pinned corpus and `expected` upstream's findings on it; a golden the corpus no longer has with
 * its sha256 (`file` undefined), or that upstream's own parser now rejects (`expected` null), fails, since it can no
 * longer be checked. A golden that differs where the engine could not fully read the file is an engine gap: it does not
 * fail the rule, since the difference is the engine's, and it is judged again on every run.
 */
export function goldensGate(
  engine: Engine,
  adapter: PackAdapter,
  name: string,
  compiled: CompiledPolicy,
  goldens: { golden: Golden; file: CorpusFile | undefined; expected: Span[] | null }[],
): { check: Check; engineCheck: Check; results: GoldenResult[]; unparsed: UnparsedFile[] } {
  const unparsed: UnparsedFile[] = [];
  const results = goldens.map(({ golden, file, expected: findings }): GoldenResult => {
    const base = { task: golden.task, path: golden.path, expected: findings?.length ?? 0 };
    if (file === undefined) {
      return { ...base, status: "mismatch", reason: `the pinned corpus has no ${golden.task}/${golden.path} with sha256 ${golden.sha256}`, actual: 0, missing: [], extra: [], failures: [] };
    }
    if (findings === null) {
      return { ...base, status: "mismatch", reason: "upstream's own parser rejects it at the pinned upstream, so it has no expected answer", actual: 0, missing: [], extra: [], failures: [] };
    }
    const language = engine.rerail.dialectForPath(file.path);
    if (language === undefined || !compiled.inScope(file.path)) {
      const unreachable = language === undefined ? `RERAIL has no dialect for ${extname(file.path) || "files without an extension"}` : "the file is outside the shipped document's scope";
      return findings.length === 0
        ? { ...base, status: "agree", reason: `${unreachable}, and upstream reports nothing in it`, actual: 0, missing: [], extra: [], failures: [] }
        : { ...base, status: "mismatch", reason: `${unreachable}, but upstream reports ${findings.length} finding(s) in it`, actual: 0, missing: findings, extra: [], failures: [] };
    }
    const evaluation = compiled.evaluateFile(file.path, language, file.source);
    const actual = evaluation.violations.map((violation) => adapter.toSpan(name, file.source, violation.span.startByte, violation.span.endByte));
    const missing = unmatched(findings, actual);
    const extra = unmatched(actual, findings);
    const blocking = evaluation.failures.filter((failure) => failure.code !== "source-partial");
    const failures = evaluation.failures.map((failure) => `${failure.code}: ${failure.message}`);
    const parseFailure = evaluation.failures.find((failure) => PARSE_FAILURE_CODES.has(failure.code));
    if (parseFailure !== undefined) {
      unparsed.push({ gate: "matches-upstream-tests", file: `${file.task}/${file.path}`, path: file.path, source: file.source, script: false, engine: `${parseFailure.code}: ${parseFailure.message}` });
    }
    const result = { ...base, actual: actual.length, missing, extra, failures };
    if (missing.length === 0 && extra.length === 0 && blocking.length === 0) return { ...result, status: "agree", reason: "identical findings" };
    const difference = [
      ...missing.slice(0, 3).map((span) => `missing ${formatSpan(span)}`),
      ...extra.slice(0, 3).map((span) => `extra ${formatSpan(span)}`),
      ...blocking.slice(0, 1).map((failure) => `engine ${failure.code}`),
    ].join(", ");
    if (evaluation.failures.some((failure) => ENGINE_GAP_CODES.has(failure.code))) return { ...result, status: "engine-gap", reason: `engine gap: ${difference}` };
    return { ...result, status: "mismatch", reason: difference };
  });
  const mismatches = results.filter((result) => result.status === "mismatch");
  const engineGaps = results.filter((result) => result.status === "engine-gap").length;
  return {
    check: {
      id: "identical-on-goldens",
      ok: mismatches.length === 0,
      detail: results.length === 0
        ? "the rule has no goldens"
        : mismatches.length > 0
          ? `${mismatches.length} of ${results.length} golden(s) differ, e.g. ${mismatches[0]!.task}/${mismatches[0]!.path}: ${mismatches[0]!.reason}${engineGaps === 0 ? "" : `; ${engineGaps} engine gap(s)`}`
          : engineGaps === 0
            ? `identical on all ${results.length} golden(s)`
            : `identical on ${results.length - engineGaps} of ${results.length} golden(s); the engine could not fully read the other ${engineGaps} (engine gaps)`,
    },
    engineCheck: engineReadsGoldensCheck(results),
    results,
    unparsed,
  };
}

/** How many engine-gap goldens engine-reads-goldens names in its detail. */
const ENGINE_GAP_SAMPLE = 3;

/**
 * matches-upstream-tests's engine-reads-goldens check: a golden that differs where the engine could not fully read the
 * file (an engine gap) fails the rule. The difference is the engine's to fix, but the rule cannot ship until it is.
 */
export function engineReadsGoldensCheck(results: Pick<GoldenResult, "task" | "path" | "status">[]): Check {
  const gaps = results.filter((result) => result.status === "engine-gap");
  if (gaps.length === 0) {
    return { id: "engine-reads-goldens", ok: true, detail: results.length === 0 ? "the rule has no goldens" : `the engine fully read every differing golden of ${results.length}` };
  }
  const first = gaps.slice(0, ENGINE_GAP_SAMPLE).map((gap) => `${gap.task}/${gap.path}`).join(", ");
  return {
    id: "engine-reads-goldens",
    ok: false,
    detail: `${gaps.length} of ${results.length} golden(s) differ where the engine could not fully read the file (${first}${gaps.length > ENGINE_GAP_SAMPLE ? ", …" : ""}): the engine could not fully read it; the fix belongs in the engine (NER-1764), and the rule cannot ship until then`,
  };
}

/** A judged case reduced to what stays-silent reads: how many findings upstream reports, and whether the rule agrees. */
interface JudgedCase {
  expected: number;
  status: string;
}

/**
 * matches-upstream-tests's stays-silent check, the counterpart of fires: the golden test proving a shipped rule stays
 * silent. At least one case where upstream reports nothing and the rule agrees, from upstream's own test files, the
 * rule's goldens, or matches-real-code's negative files (those richest in the rule's own constructs, so near misses).
 */
export function staysSilentCheck(upstreamTests: JudgedCase[], goldens: JudgedCase[], realCode: (JudgedCase & { kind: "positive" | "negative" })[]): Check {
  const silent = (cases: JudgedCase[]) => cases.filter((testCase) => testCase.expected === 0 && testCase.status === "agree").length;
  const counts = { tests: silent(upstreamTests), goldens: silent(goldens), realCode: silent(realCode.filter((testCase) => testCase.kind === "negative")) };
  const total = counts.tests + counts.goldens + counts.realCode;
  return {
    id: "stays-silent",
    ok: total > 0,
    detail: `${total} case(s) where upstream reports nothing and the rule agrees: ${counts.tests} upstream test file(s), ${counts.goldens} golden(s), ${counts.realCode} real-code negative(s)`,
  };
}
