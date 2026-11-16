// The shippability bar's verdict logic (scripts/ship/shipbar.ts): pure functions over measured runs and findings, the
// same for every candidate. A candidate passes a metric when its median is at most 1.10 times main's, keeps every
// finding main gets right and adds no false positive main does not have, completes every run main completes, and
// ships every rule main ships.

/** The tolerance, in percent: a candidate figure fails when it is more than this much above main's. */
export const TOLERANCE_PERCENT = 10;

export type Status = "pass" | "fail" | "not-compared";

/** The median of `values`: the middle one, or the mean of the middle two. */
export function median(values: readonly number[]): number {
  if (values.length === 0) throw new Error("median of no values");
  const sorted = [...values].sort((a, b) => a - b);
  const middle = sorted.length >> 1;
  return sorted.length % 2 === 1 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
}

/** The run-to-run spread of `values`: (max - min) / median, 0 for one value or a zero median. */
export function spread(values: readonly number[]): number {
  if (values.length < 2) return 0;
  const m = median(values);
  return m === 0 ? 0 : (Math.max(...values) - Math.min(...values)) / m;
}

/** Whether `candidate` is within the tolerance of `main`: candidate <= main * (1 + TOLERANCE_PERCENT / 100), exactly. */
export function withinTolerance(main: number, candidate: number, tolerancePercent = TOLERANCE_PERCENT): boolean {
  // Scaled to integers' arithmetic so 1.10 times main is the bar itself, not a float next to it.
  return candidate * 100 <= main * (100 + tolerancePercent);
}

export interface MetricVerdict {
  metric: string;
  status: Status;
  /** Medians of the completed runs; null when that side has none. */
  main: number | null;
  candidate: number | null;
  /** candidate / main. */
  ratio: number | null;
  spreadMain: number | null;
  spreadCandidate: number | null;
  /** |ratio - 1.10| is at most the larger spread: the verdict could go either way between runs. */
  withinSpread: boolean;
  reason: string;
}

/**
 * One metric of one pack and corpus: each side's median of its completed runs, compared at the tolerance. A side with
 * no completed run leaves the metric not compared (completion carries that failure).
 */
export function compareMetric(metric: string, main: readonly number[], candidate: readonly number[], tolerancePercent = TOLERANCE_PERCENT): MetricVerdict {
  const base = { metric, spreadMain: main.length > 0 ? spread(main) : null, spreadCandidate: candidate.length > 0 ? spread(candidate) : null };
  if (main.length === 0 || candidate.length === 0) {
    const missing = main.length === 0 && candidate.length === 0 ? "neither side" : main.length === 0 ? "main" : "the candidate";
    return { ...base, status: "not-compared", main: main.length > 0 ? median(main) : null, candidate: candidate.length > 0 ? median(candidate) : null, ratio: null, withinSpread: false, reason: `${missing} completed no run` };
  }
  const m = median(main);
  const c = median(candidate);
  const ratio = m === 0 ? (c === 0 ? 1 : Number.POSITIVE_INFINITY) : c / m;
  const pass = withinTolerance(m, c, tolerancePercent);
  const bar = 1 + tolerancePercent / 100;
  const noise = Math.max(base.spreadMain ?? 0, base.spreadCandidate ?? 0);
  const withinSpread = Number.isFinite(ratio) && Math.abs(ratio - bar) <= noise;
  const reason = pass ? "" : `${ratio.toFixed(3)}x main (bar ${bar.toFixed(2)}x)`;
  return { ...base, status: pass ? "pass" : "fail", main: m, candidate: c, ratio, withinSpread, reason };
}

// ---------------------------------------------------------------- findings

/** A finding's key: (rule, path, line, code), as one string. Two findings at one key are one. */
export function findingKey(rule: string, path: string, line: number, code: string): string {
  return `${rule}\u0000${path}\u0000${line}\u0000${code}`;
}

export function parseKey(key: string): { rule: string; path: string; line: number; code: string } {
  const [rule = "", path = "", line = "0", code = ""] = key.split("\u0000");
  return { rule, path, line: Number(line), code };
}

/** One entry of a pack's divergences.json: a known difference from upstream at (rule, path, line). */
export interface Divergence {
  rule: string;
  path: string;
  line: number;
  kind: string;
  source?: string;
}

export interface Score {
  tp: Set<string>;
  fp: Set<string>;
  fn: Set<string>;
  /** Unmatched keys a divergence explains: neither FP nor FN. */
  explainedFp: number;
  explainedFn: number;
}

/**
 * One side's findings against the upstream recording: keys in both are true positives; a key only the side reports is
 * a false positive unless a divergence of its rule at its (path, line) explains it (`rerail-unknown` explains only a
 * miss); a recorded key the side misses is a false negative unless any divergence there explains it.
 */
export function scoreFindings(expected: ReadonlySet<string>, actual: ReadonlySet<string>, divergences: readonly Divergence[]): Score {
  const explainsMiss = new Set<string>();
  const explainsExtra = new Set<string>();
  for (const d of divergences) {
    const at = `${d.rule}\u0000${d.path}\u0000${d.line}`;
    explainsMiss.add(at);
    if (d.kind !== "rerail-unknown") explainsExtra.add(at);
  }
  const at = (key: string): string => key.slice(0, key.lastIndexOf("\u0000"));
  const score: Score = { tp: new Set(), fp: new Set(), fn: new Set(), explainedFp: 0, explainedFn: 0 };
  for (const key of actual) {
    if (expected.has(key)) score.tp.add(key);
    else if (explainsExtra.has(at(key))) score.explainedFp += 1;
    else score.fp.add(key);
  }
  for (const key of expected) {
    if (actual.has(key)) continue;
    if (explainsMiss.has(at(key))) score.explainedFn += 1;
    else score.fn.add(key);
  }
  return score;
}

export interface FindingsVerdict {
  status: Status;
  /** Keys main gets right that the candidate does not report. */
  lostTp: string[];
  /** False positives of the candidate that main does not have. */
  newFp: string[];
  reason: string;
}

/** The set-wise bar: the candidate keeps every true positive of main's and adds no false positive main lacks. */
export function compareFindings(main: Score, candidate: Score): FindingsVerdict {
  const lostTp = [...main.tp].filter((key) => !candidate.tp.has(key)).sort();
  const newFp = [...candidate.fp].filter((key) => !main.fp.has(key)).sort();
  const reasons = [];
  if (lostTp.length > 0) reasons.push(`${lostTp.length} true positive(s) of main lost`);
  if (newFp.length > 0) reasons.push(`${newFp.length} false positive(s) main does not have`);
  return { status: reasons.length === 0 ? "pass" : "fail", lostTp, newFp, reason: reasons.join("; ") };
}

/** Real code has no truth: how far the two sides' keys agree, reported and never gating. */
export function agreement(main: ReadonlySet<string>, candidate: ReadonlySet<string>): { both: number; mainOnly: string[]; candidateOnly: string[] } {
  let both = 0;
  const mainOnly: string[] = [];
  for (const key of main) {
    if (candidate.has(key)) both += 1;
    else mainOnly.push(key);
  }
  const candidateOnly = [...candidate].filter((key) => !main.has(key));
  return { both, mainOnly: mainOnly.sort(), candidateOnly: candidateOnly.sort() };
}

// ---------------------------------------------------------------- completion

/** One process's outcome: completed, or why and where it stopped. */
export interface RunOutcome {
  completed: boolean;
  reason?: string;
}

export type CompletionKind = "both-completed" | "candidate-failed" | "both-failed" | "main-failed";

export interface CompletionVerdict {
  status: Status;
  kind: CompletionKind;
  reason: string;
}

/**
 * Every run main completes, the candidate must complete under the same caps. `requested` is how many runs each side
 * was asked for; runs not made (a side stops after its first failure) count as failed with the first failure's reason.
 * Where main fails too, the pair is reported as both-failed and does not fail the candidate.
 */
export function compareCompletion(main: readonly RunOutcome[], candidate: readonly RunOutcome[], requested: number): CompletionVerdict {
  const failure = (runs: readonly RunOutcome[]): string | null => {
    const failed = runs.find((r) => !r.completed);
    if (failed !== undefined) return failed.reason ?? "did not complete";
    return runs.length < requested ? `${requested - runs.length} of ${requested} runs not made` : null;
  };
  const m = failure(main);
  const c = failure(candidate);
  if (m === null && c === null) return { status: "pass", kind: "both-completed", reason: "" };
  if (m === null) return { status: "fail", kind: "candidate-failed", reason: c! };
  if (c === null) return { status: "pass", kind: "main-failed", reason: `main: ${m}` };
  return { status: "pass", kind: "both-failed", reason: `main: ${m}; candidate: ${c}` };
}

/** Rules whose evaluation failed on the candidate (a compile error or an engine failure) and not on main. */
export function newRuleFailures(main: ReadonlyMap<string, string>, candidate: ReadonlyMap<string, string>): { rule: string; reason: string }[] {
  return [...candidate].filter(([rule]) => !main.has(rule)).map(([rule, reason]) => ({ rule, reason })).sort((a, b) => (a.rule < b.rule ? -1 : 1));
}

// ---------------------------------------------------------------- coverage

/**
 * Every rule main ships needs a runnable counterpart on the candidate. `candidate` maps a rule id to null (runnable)
 * or the reason it cannot run (its `.rr` fails `check-rr`, …); a rule main ships that `candidate` lacks has none.
 */
export function missingRules(mainRules: readonly string[], candidate: ReadonlyMap<string, string | null>): { rule: string; reason: string }[] {
  const missing: { rule: string; reason: string }[] = [];
  for (const rule of [...mainRules].sort()) {
    if (!candidate.has(rule)) missing.push({ rule, reason: "no counterpart on the candidate" });
    else {
      const reason = candidate.get(rule);
      if (reason !== null && reason !== undefined) missing.push({ rule, reason });
    }
  }
  return missing;
}
