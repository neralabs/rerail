import type { CompiledPolicy, Engine } from "../engine.js";
import { byText } from "../goldens.js";
import { codeWords, policyLiterals } from "../policy-literals.js";
import { unmatched } from "../spans.js";
import type { Check, PackAdapter, Span, UpstreamFile } from "../types.js";
import { PARSE_FAILURE_CODES, type UnparsedFile } from "./matches-upstream-tests.js";

/** matches-real-code keeps up to this many positive and this many negative cases per rule. */
export const CASES_PER_KIND = 5;

/** How matches-real-code judges, recorded in every report. */
export const MATCHES_REAL_CODE_METHOD = {
  corpus: "the files each SWE Atlas/Evo task's reference solution (its vetted end-of-task code diff, pinned by sha256) leaves behind, at the task's base commit; and every file under the pinned paths of each corpus the pack's rerail pack pins in its conformance manifests (a repository at a commit), named rerail:<corpus> in place of a task",
  "ground truth": "the upstream linter on the same file, configured as upstream recommends",
  eligible: "files the shipped document applies to and upstream's own parser reads without syntax errors",
  cases: "up to 5 positive files (upstream reports the rule) in task and path order, and up to 5 negative files ranked by how many of the rule's own constructs they contain",
  agreement: "the same findings as upstream on every case, start and end of every span less any end part matches-upstream-tests leaves out for the rule, with nothing missing and nothing extra, and no engine failure",
  "end to end": "matches-real-code never stops a rule and never decides whether it starts on; a file where the rule and upstream differ means matches-upstream-tests was missing that case, so every eligible file where they differ, among its cases or elsewhere, that is not a golden yet is promoted to a golden test in the tool-pack's goldens.tsv and judged by this run's matches-upstream-tests; a golden's expected answer is the pinned upstream's findings on its file",
  report: "per rule, the cases, and how many other eligible files differ with the first 5 of them",
  excused: "nothing excuses a difference: a corpus file is real code, so a file the engine cannot parse is a mismatch, and becomes a golden that matches-upstream-tests reports as an engine gap until the engine reads it",
  "gap candidates": "every eligible file the engine cannot fully parse is read by the same independent parsers as matches-upstream-tests's",
} as const;


export interface RealCodeCase {
  task: string;
  path: string;
  kind: "positive" | "negative";
  status: "agree" | "mismatch";
  expected: number;
  actual: number;
  missing: Span[];
  extra: Span[];
  failures: string[];
}

export interface MatchesRealCodeResult {
  checks: Check[];
  eligible: number;
  positives: number;
  cases: RealCodeCase[];
  /** Eligible files outside the selected cases where the rule and upstream disagree; each becomes a golden too. */
  disagreementsElsewhere: { task: string; path: string; expected: number; actual: number }[];
  /** Eligible files the engine could not fully parse; run.ts asks independent parsers about them. */
  unparsed: UnparsedFile[];
}

/**
 * The words that name a rule's constructs: the words its patterns, regexes and values spell, and the keywords its
 * grammar node kinds are named after (`debugger_statement` gives `debugger`).
 */
export function constructWords(document: { clauses?: unknown }, keywords: Set<string>): Set<string> {
  const found = new Set(policyLiterals(document).filter((literal) => literal.role !== "context").map((literal) => literal.token));
  const kinds = (value: unknown): void => {
    if (Array.isArray(value)) value.forEach(kinds);
    else if (value !== null && typeof value === "object") {
      for (const [field, child] of Object.entries(value as Record<string, unknown>)) {
        if (field === "kind" && typeof child === "string") {
          for (const part of child.split("_")) if (keywords.has(part)) found.add(part);
        } else {
          kinds(child);
        }
      }
    }
  };
  kinds(document.clauses);
  return found;
}

function constructScore(source: string, constructs: Set<string>): number {
  const present = new Set(codeWords(source));
  return [...constructs].filter((word) => present.has(word)).length;
}

/**
 * matches-real-code: on real code from SWE Atlas/Evo tasks and the pack's pinned corpora, the rule must agree with
 * upstream on every case. Cases are chosen from upstream's verdicts alone, so they do not move when a port changes:
 * positives in task and path order, and negatives that contain the rule's own constructs first. It never stops a rule:
 * each mismatching case becomes a golden of matches-upstream-tests (run.ts).
 */
export function matchesRealCodeGate(
  engine: Engine,
  adapter: PackAdapter,
  compiled: CompiledPolicy,
  name: string,
  document: { clauses?: unknown },
  files: UpstreamFile[],
): MatchesRealCodeResult {
  const order = (a: UpstreamFile, b: UpstreamFile) => byText(a.file.task, b.file.task) || byText(a.file.path, b.file.path);
  const eligible = files.filter((entry) => entry.excluded === null && compiled.inScope(entry.file.path)
    && engine.rerail.dialectForPath(entry.file.path) !== undefined).sort(order);
  const expectedOf = (entry: UpstreamFile) => entry.findings.get(name) ?? [];
  const constructs = constructWords(document, adapter.keywords);
  const positives = eligible.filter((entry) => expectedOf(entry).length > 0);
  const negatives = eligible.filter((entry) => expectedOf(entry).length === 0)
    .map((entry) => ({ entry, score: constructScore(entry.file.source, constructs) }))
    .sort((a, b) => b.score - a.score || order(a.entry, b.entry))
    .map(({ entry }) => entry);
  const selected = [
    ...positives.slice(0, CASES_PER_KIND).map((entry) => ({ entry, kind: "positive" as const })),
    ...negatives.slice(0, CASES_PER_KIND).map((entry) => ({ entry, kind: "negative" as const })),
  ];

  const unparsed = new Map<UpstreamFile, UnparsedFile>();
  const judge = (entry: UpstreamFile) => {
    const evaluation = compiled.evaluateFile(entry.file.path, engine.rerail.dialectForPath(entry.file.path)!, entry.file.source);
    const actual = evaluation.violations.map((violation) => adapter.toSpan(name, entry.file.source, violation.span.startByte, violation.span.endByte));
    const expected = expectedOf(entry);
    const blocking = evaluation.failures.filter((failure) => failure.code !== "source-partial");
    const parseFailure = evaluation.failures.find((failure) => PARSE_FAILURE_CODES.has(failure.code));
    if (parseFailure !== undefined && !unparsed.has(entry)) {
      unparsed.set(entry, { gate: "matches-real-code", file: `${entry.file.task}/${entry.file.path}`, path: entry.file.path, source: entry.file.source, script: false, engine: `${parseFailure.code}: ${parseFailure.message}` });
    }
    return {
      expected,
      actual,
      missing: unmatched(expected, actual),
      extra: unmatched(actual, expected),
      failures: evaluation.failures.map((failure) => `${failure.code}: ${failure.message}`),
      blocking: blocking.length,
    };
  };

  const cases: RealCodeCase[] = selected.map(({ entry, kind }) => {
    const verdict = judge(entry);
    const agree = verdict.missing.length === 0 && verdict.extra.length === 0 && verdict.blocking === 0;
    return {
      task: entry.file.task,
      path: entry.file.path,
      kind,
      status: agree ? "agree" : "mismatch",
      expected: verdict.expected.length,
      actual: verdict.actual.length,
      missing: verdict.missing,
      extra: verdict.extra,
      failures: verdict.failures,
    };
  });
  const chosen = new Set(selected.map(({ entry }) => entry));
  const disagreementsElsewhere = eligible.filter((entry) => !chosen.has(entry)).flatMap((entry) => {
    const verdict = judge(entry);
    return verdict.missing.length === 0 && verdict.extra.length === 0 && verdict.blocking === 0
      ? []
      : [{ task: entry.file.task, path: entry.file.path, expected: verdict.expected.length, actual: verdict.actual.length }];
  });

  const mismatches = cases.filter((testCase) => testCase.status === "mismatch");
  return {
    checks: [{
      id: "agrees-on-real-code",
      ok: mismatches.length === 0,
      detail: mismatches.length === 0
        ? `agrees with upstream on all ${cases.length} case(s) (${Math.min(positives.length, CASES_PER_KIND)} positive) from ${eligible.length} eligible file(s)`
        : `${mismatches.length} of ${cases.length} case(s) differ, e.g. ${mismatches[0]!.kind} ${mismatches[0]!.task}/${mismatches[0]!.path}: upstream ${mismatches[0]!.expected}, rule ${mismatches[0]!.actual}`,
    }],
    eligible: eligible.length,
    positives: positives.length,
    cases,
    disagreementsElsewhere,
    unparsed: [...unparsed.values()],
  };
}

/**
 * Every eligible file where the rule and upstream differ, among the selected cases and elsewhere, in task and path
 * order: each is a case matches-upstream-tests was missing, so each becomes a golden.
 */
export function differingFiles(result: Pick<MatchesRealCodeResult, "cases" | "disagreementsElsewhere">): { task: string; path: string }[] {
  const byPlace = (a: { task: string; path: string }, b: { task: string; path: string }) => (a.task < b.task ? -1 : a.task > b.task ? 1 : a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
  return [
    ...result.cases.filter((testCase) => testCase.status === "mismatch"),
    ...result.disagreementsElsewhere,
  ].map(({ task, path }) => ({ task, path })).sort(byPlace);
}
