// The ship / no-ship report, report.json (schema ship-no-ship/report/v2), built from what shipbar.ts measured. Per
// pack, the three slices side by side: A (all rules × all files, per corpus), B (one rule × all files: the fixed cost and
// every rule alone, per corpus) and C (all rules × one file: three files of the real-code corpus). Every item keeps its
// raw runs, ratio and status under a stable id, so runs compare across nights; each failing item names the phase that
// moved (its locator), and each level names its biggest contributor one level down, so a reader can stop at the pack
// table. A figure that fails at the median and passes its confirmation runs is noisy: reported, not counted. A pack
// missing any slice is incomplete and never passes; a run narrowed by --packs or --corpus is PARTIAL, never PASS. A
// pack whose verdict does not count (the `.rerail` side's, without an equivalence.json saying equivalent) is judged
// alike and reported only: the candidate's counted status, the one the exit status reads, leaves it out. Pure
// functions; report.ts renders the Markdown.

import { methodGaps, type RunMethod } from "./platform.js";
import type { Candidate, CellVerdict, Corpus, LoadSummary, MeasuredMetric, OneFileVerdict, OneRuleTiming, OneRuleVerdict, PackVerdict, RuleTiming, SameFindings, Verdict } from "./shipbar.js";
import { type CellCost, KNOWN_GAPS, type RuleMultiple, ruleMultiples } from "./metric.js";
import { type FileLabel, median, RR_RULE_TIME, RULE_ALERT_RATIO, RULE_FLOOR_MS, ruleSliceStats, SLICES_TOLERANCE_PERCENT, spread, type TimeVerdict } from "./verdict.js";
import type { WaitsSection } from "./waits-model.js";

export const SCHEMA = "ship-no-ship/report/v2";

/** noisy: failed at the median and passed its confirmation runs; reported and not counted. */
export type ItemStatus = "pass" | "fail" | "noisy" | "not_compared";
export type Slice = "A" | "B" | "C";
export type VerdictStatus = "PASS" | "FAIL" | "INCOMPLETE" | "PARTIAL";

export interface SideStats {
  runs: number[];
  median: number | null;
  spread_pct: number | null;
}

export interface MetricItem {
  id: string;
  metric: string;
  unit: "ms" | "mb";
  baseline: SideStats;
  candidate: SideStats;
  ratio: number | null;
  /** The ratio's distance from the bar is within the larger side's run-to-run spread. */
  within_spread: boolean;
  status: ItemStatus;
  confirm: { baseline: SideStats; candidate: SideStats; ratio: number | null; still_fails: boolean } | null;
  /** The phase that moved, where the metric fails at its first runs. */
  locator: string | null;
}

/** A rule's added time alone, and its share of the corpus's total. */
export interface Attribution {
  rule: string;
  baseline_ms: number | null;
  candidate_ms: number | null;
  added_ms: number | null;
  /** added_ms over the total added time, when that total is positive. */
  share: number | null;
}

/** A slice B time: a rule's compile + evaluate, or one of the fixed cost's figures. */
export interface TimeItem {
  name: string;
  baseline_ms: number | null;
  candidate_ms: number | null;
  ratio: number | null;
  /** Over the bar at the first runs, before confirmation and the floor. */
  over_before: boolean;
  confirm: { baseline_ms: number; candidate_ms: number; ratio: number } | null;
  /** fail: still over after confirmation by at least the floor; under_floor: over by less (counted as a pass). */
  time_status: "pass" | "fail" | "noisy" | "under_floor" | "not_compared";
  status: ItemStatus;
}

export interface SameItem {
  status: ItemStatus;
  both: number;
  baseline_only: number;
  candidate_only: number;
  samples: string[];
}

export interface RuleItem extends Attribution {
  /** The rule's time alone: compile + evaluate, or the `.rerail` side's net wall (verdict.ts RR_RULE_TIME). */
  time: TimeItem;
  status: ItemStatus;
  locator: string | null;
  completion: { status: ItemStatus; kind: string; reason: string };
  findings: SameItem | null;
  rule_failure: string | null;
  wall_ms: { baseline: number | null; candidate: number | null };
  rss_mb: { baseline: number | null; candidate: number | null };
  /** metric.ts: its time alone over the corpus's median rule's, each side; informational. */
  multiple?: RuleMultiple["multiple"];
  flags?: string[];
}

export interface LoadItem {
  runs: number;
  load_median: number | null;
  load_max: number | null;
  sibling_busy_max: number | null;
}

export interface RuleStatsItem {
  rules: number;
  compared: number;
  median_ratio: number | null;
  p95_ratio: number | null;
  /** Over 1.15× at the first runs, before confirmation and the 20 ms floor. */
  over_bar_before: string[];
  /** Still over after confirmation and moved by at least the floor: the rules that fail. */
  failing: string[];
  /** Over 1.5× and moved by at least the floor. */
  over_alert: string[];
  worst: { rule: string; ratio: number; baseline_ms: number; candidate_ms: number }[];
}

/** Slice B on one corpus. */
export interface OneRuleItem {
  id: string;
  corpus: Corpus;
  status: ItemStatus;
  /** Why it is not compared. */
  reason: string | null;
  unmeasured: string[];
  /** What each rule's judged time is (ruleTimeLabel): compile + evaluate, or the `.rerail` side's net wall. */
  rule_time: string;
  runs_per_side: number;
  /** The empty rule set: cold wall, load, compile and evaluate. */
  fixed: { status: ItemStatus; times: TimeItem[]; locator: string | null; completion: { status: ItemStatus; kind: string; reason: string } } | null;
  /** Every rule alone, the largest added time first. */
  rules: RuleItem[];
  total_added_ms: number;
  stats: RuleStatsItem;
  findings: { identical: number; differing: number; not_compared: number };
  /** The rules that carry the corpus's added time. */
  hint: string | null;
  load: LoadItem;
}

/** Slice C: one file. */
export interface OneFileItem {
  id: string;
  label: FileLabel;
  path: string;
  bytes: number;
  status: ItemStatus;
  reason: string | null;
  completion: { status: ItemStatus; kind: string; reason: string; stopped_on: string | null } | null;
  metrics: MetricItem[];
  findings: SameItem | null;
  rule_failures: { rule: string; reason: string }[];
  load: { baseline: (number | null)[]; candidate: (number | null)[] };
  sibling_busy_max: number | null;
}

export interface Counts {
  tp: number;
  fp: number;
  fn: number;
  explained_fp: number;
  explained_fn: number;
}

export interface CellItem {
  id: string;
  candidate: Candidate;
  pack: string;
  corpus: Corpus;
  status: ItemStatus;
  /** Why the cell is not compared, or why it fails without a measurement. */
  reason: string | null;
  rules: { baseline: number; candidate: number };
  completion: { status: ItemStatus; kind: string; reason: string; stopped_on: string | null } | null;
  metrics: MetricItem[];
  findings: { status: ItemStatus; reason: string; baseline: Counts; candidate: Counts; lost_tp: number; new_fp: number; samples: string[] } | null;
  agreement: { both: number; baseline_only: number; candidate_only: number; samples: string[] } | null;
  rule_failures: { rule: string; reason: string }[];
  /** The biggest contributor one level down: the rules behind a slowdown (slice B), where a run stopped, a differing finding. */
  hint: string | null;
  load: { baseline: (number | null)[]; candidate: (number | null)[] };
  /** The busiest idle sibling CPU over the cell's runs; null when none was pinned. */
  sibling_busy_max: number | null;
  /** What the cell's runs lacked against pinned, kernel-capped runs (platform.ts methodGaps); empty on Linux. */
  method_gaps: string[];
  reproduce: string;
  /** metric.ts: start-up, net times, phases and workload; reported beside the gross metrics, never a status. */
  cost: CellCost | null;
  /** The caps the cell's runs ran under, both sides alike: the pack's `--pack-cap`, else the defaults. */
  caps: { memory: string[]; time_s: number[] };
  /** `.rerail`: the whole-pack run did not complete, so the candidate's metrics and findings are a chunked run's sums. */
  chunked: { size: number; chunks: number; rules_run: number; rules_of: number; failed_chunks: { chunk: number; rules: string; reason: string }[] } | null;
  /** `.rerail`: how far the failed whole-pack run got (policies started, elapsed, peak RSS). */
  progress: { started: number; of: number; wall_ms: number; rss_kb: number } | null;
  /** `.rerail`: whether run-policies ran the pack's policies combined (groups of several) or alone; null: not reported. */
  grouping: { policies: number; combined: number; alone: number; largest: number } | null;
  /** `.rerail`: what the engine printed about running policies alone instead of together. */
  fallback: string[];
}

export type SliceStatus = ItemStatus | "missing";

export interface PackSummary {
  /** Each corpus's wall-cold ratio and status: the pack table's slice A columns. */
  wall_cold: Partial<Record<Corpus, { ratio: number | null; status: ItemStatus; completed: boolean; net_ratio: number | null }>>;
  /** The failing metric with the highest ratio, else the highest ratio of any metric (slice A). */
  worst: { id: string; corpus: Corpus; metric: string; ratio: number; status: ItemStatus } | null;
  findings: "same" | "no_worse" | "worse" | "not_compared";
  completed: boolean;
  hint: string | null;
  /** Each slice's status, and the figures the pack table shows of B and C. */
  slices: {
    A: { status: SliceStatus };
    B: { status: SliceStatus; rules: number; fixed_wall: Partial<Record<Corpus, number | null>>; stats: RuleStatsItem | null; findings: { identical: number; differing: number } };
    /** Each file's worst metric ratio and the status of its metrics (its findings apart). */
    C: { status: SliceStatus; files: { label: FileLabel; path: string; worst_ratio: number | null; metrics_status: ItemStatus; findings_same: boolean | null }[] };
  };
  /** The highest 1-minute load at any run's start and the busiest idle sibling CPU, over all three slices. */
  load: { load_max: number | null; sibling_busy_max: number | null };
}

export interface PackItem {
  id: string;
  candidate: Candidate;
  pack: string;
  status: ItemStatus | "incomplete" | "no_baseline";
  /** Whether the pack's verdict counts toward its candidate's counted status (the exit status); false: reported only. */
  counted: boolean;
  /** Why it counts or not (the `.rerail` side's equivalence.json); null where it always counts. */
  gate_reason: string | null;
  summary: PackSummary;
  coverage: { missing: { rule: string; reason: string }[]; extra: string[] };
  /** Slice A. */
  cells: CellItem[];
  one_rule: OneRuleItem[];
  one_file: OneFileItem[];
  /** The slices (or parts of them) this pack lacks: while any is listed, it never passes. */
  incomplete: MissingSlice[];
}

export interface MissingSlice {
  slice: Slice;
  reason: string;
}

export interface FailureItem {
  id: string;
  candidate: Candidate;
  pack: string;
  /** Whether its pack's verdict counts (PackItem.counted); false: reported only. */
  counted: boolean;
  slice: Slice | null;
  corpus: Corpus | null;
  item: string;
  reason: string;
  /** The phase that moved (load, compile, evaluate, store, startup), or where the run stopped. */
  locator: string | null;
  hint: string | null;
}

export interface CandidateItem {
  candidate: Candidate;
  description: string;
  /** Over every pack, counted or reported only. */
  status: VerdictStatus;
  /** Over the packs whose verdict counts: what the exit status reads; null when none counts. */
  counted_status: VerdictStatus | null;
  counts: { packs: number; failing_packs: number; noisy_packs: number; incomplete_packs: number; reported_packs: number; failing_items: number; noisy_items: number };
  failures: FailureItem[];
  noisy: FailureItem[];
  packs: PackItem[];
}

export interface Report {
  schema: typeof SCHEMA;
  run: { started_at: string | null; finished_at: string; invoked_by: string | null; command: string | null; harness_sha: string };
  machines: Verdict["machines"];
  baseline: { ref: string | null; sha: string };
  candidate: { ref: string | null; sha: string };
  inputs: Verdict["corpora"];
  bar: {
    max_ratio: number;
    runs_per_side: number;
    confirm_runs: number;
    rule_runs: number;
    file_runs: number;
    rule_floor_ms: number;
    rule_alert_ratio: number;
    caps: { memory: string[]; time_s: number[] };
    pinned_cpus: string[];
    threads: number[];
    /** Each way runs were pinned, capped and measured, how many runs it measured, and what it lacks. */
    methods: { method: RunMethod; runs: number; gaps: string[] }[];
  };
  verdict: {
    status: VerdictStatus;
    gate: Candidate[];
    /** How the run was narrowed: a partial verdict is never PASS. */
    partial: string[];
    candidates: Partial<Record<Candidate, { status: CandidateItem["status"]; counted_status: CandidateItem["counted_status"]; failing_packs: number; failing_items: number }>>;
    waits: WaitsSection["status"] | null;
  };
  candidates: CandidateItem[];
  /** The waits a user experiences (waits-model.ts), ids wait/<W>/<scenario>; null when the run measured none. */
  waits: WaitsSection | null;
  scope: { measured: string[]; not_measured: string[]; known_gaps: string[] };
}

// ---------------------------------------------------------------- statuses

/** One metric's status: a fail confirmed by its confirmation runs, or noisy when they pass. Unconfirmed fails stand. */
export function metricStatus(measured: MeasuredMetric): ItemStatus {
  const { verdict, confirm } = measured;
  if (verdict.status === "not-compared") return "not_compared";
  if (verdict.status === "pass") return "pass";
  if (confirm === null || confirm.verdict.status === "not-compared") return "fail";
  return confirm.verdict.status === "fail" ? "fail" : "noisy";
}

/** The worst of several statuses: fail, then noisy, then pass; not compared only when nothing else is there. */
export function worstStatus(statuses: readonly ItemStatus[]): ItemStatus {
  for (const status of ["fail", "noisy", "pass"] as const) if (statuses.includes(status)) return status;
  return "not_compared";
}

// ---------------------------------------------------------------- per-rule attribution

/**
 * A rule is named as a slowdown's cause when it stands out: its added time at least STANDS_OUT times the median added
 * time of the other rules, and at least MIN_SHARE of the corpus's total. At most MAX_NAMED rules are named; a slowdown
 * no rule stands out in is reported as spread over every rule.
 */
export const STANDS_OUT = 3;
export const MIN_SHARE = 0.2;
export const MAX_NAMED = 2;

/** Each rule's added time, largest first (rules a side could not run last), and the total added time. */
export function attribute(rules: readonly RuleTiming[]): { rules: Attribution[]; totalAddedMs: number } {
  const items = rules.map((r): Attribution => ({ rule: r.rule, baseline_ms: r.main, candidate_ms: r.candidate, added_ms: r.main === null || r.candidate === null ? null : r.candidate - r.main, share: null }));
  const totalAddedMs = items.reduce((sum, r) => sum + (r.added_ms ?? 0), 0);
  for (const r of items) r.share = r.added_ms !== null && totalAddedMs > 0 ? r.added_ms / totalAddedMs : null;
  items.sort((a, b) => (b.added_ms ?? Number.NEGATIVE_INFINITY) - (a.added_ms ?? Number.NEGATIVE_INFINITY) || (a.rule < b.rule ? -1 : 1));
  return { rules: items, totalAddedMs };
}

/** The rule-level hint of a slower corpus: the rules that stand out, or how much each rule moved when none does. */
export function ruleHint(attributed: { rules: readonly Attribution[]; total_added_ms: number }): string | null {
  const timed = attributed.rules.filter((r) => r.added_ms !== null);
  if (timed.length === 0) return null;
  const total = attributed.total_added_ms;
  if (total <= 0) return `no rule slower in total (${signedMs(total)} over ${timed.length} rules)`;
  const named = timed
    .filter((r) => {
      const others = timed.filter((o) => o !== r).map((o) => o.added_ms!);
      return r.added_ms! > 0 && (r.share ?? 0) >= MIN_SHARE && (others.length === 0 || r.added_ms! >= STANDS_OUT * median(others));
    })
    .slice(0, MAX_NAMED);
  if (named.length > 0) return `${named.map((r) => `\`${r.rule}\` ${signedMs(r.added_ms!)}`).join(", ")} of ${signedMs(total)}`;
  return `spread over ${timed.length} rules: ${signedMs(median(timed.map((r) => r.added_ms!)))} each (median), largest \`${timed[0]!.rule}\` ${signedMs(timed[0]!.added_ms!)}; ${signedMs(total)} in all`;
}

export function signedMs(ms: number): string {
  const sign = ms < 0 ? "−" : "+";
  return `${sign}${formatMs(Math.abs(ms))}`;
}

export function formatMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return ms < 10_000 ? `${(ms / 1000).toFixed(2)} s` : `${(ms / 1000).toFixed(1)} s`;
}

// ---------------------------------------------------------------- building the report

function stats(runs: readonly number[]): SideStats {
  return { runs: [...runs], median: runs.length > 0 ? median(runs) : null, spread_pct: runs.length > 1 ? Math.round(spread(runs) * 1000) / 10 : null };
}

const KB_PER_MB = 1024;

function metricItem(prefix: string, measured: MeasuredMetric): MetricItem {
  const rss = measured.verdict.metric.startsWith("rss");
  const scale = (runs: readonly number[]): number[] => (rss ? runs.map((kb) => Math.round((kb / KB_PER_MB) * 10) / 10) : [...runs]);
  const { verdict, confirm } = measured;
  return {
    id: `${prefix}/${verdict.metric}`,
    metric: verdict.metric,
    unit: rss ? "mb" : "ms",
    baseline: stats(scale(measured.mainRuns)),
    candidate: stats(scale(measured.candidateRuns)),
    ratio: verdict.ratio,
    within_spread: verdict.withinSpread,
    status: metricStatus(measured),
    confirm: confirm === null ? null : { baseline: stats(scale(confirm.mainRuns)), candidate: stats(scale(confirm.candidateRuns)), ratio: confirm.verdict.ratio, still_fails: confirm.verdict.status === "fail" },
    locator: measured.locator ?? null,
  };
}

function counts(c: { tp: number; fp: number; fn: number; explainedFp: number; explainedFn: number }): Counts {
  return { tp: c.tp, fp: c.fp, fn: c.fn, explained_fp: c.explainedFp, explained_fn: c.explainedFn };
}

function completionStatus(status: "pass" | "fail" | "not-compared"): ItemStatus {
  return status === "fail" ? "fail" : status === "pass" ? "pass" : "not_compared";
}

function sameItem(same: SameFindings | null): SameItem | null {
  if (same === null) return null;
  return { status: same.mainOnly === 0 && same.candidateOnly === 0 ? "pass" : "fail", both: same.both, baseline_only: same.mainOnly, candidate_only: same.candidateOnly, samples: same.samples };
}

function timeItem(t: TimeVerdict): TimeItem {
  return {
    name: t.name,
    baseline_ms: t.main,
    candidate_ms: t.candidate,
    ratio: t.ratio,
    over_before: t.overBefore,
    confirm: t.confirm === null ? null : { baseline_ms: t.confirm.main, candidate_ms: t.confirm.candidate, ratio: t.confirm.ratio },
    time_status: t.status === "not-compared" ? "not_compared" : t.status === "under-floor" ? "under_floor" : t.status,
    status: t.status === "not-compared" ? "not_compared" : t.status === "under-floor" ? "pass" : t.status,
  };
}

function loadItem(load: LoadSummary): LoadItem {
  return { runs: load.runs, load_median: load.loadMedian, load_max: load.loadMax, sibling_busy_max: load.siblingBusyMax };
}

function ruleStatsItem(times: readonly TimeVerdict[]): RuleStatsItem {
  const s = ruleSliceStats(times);
  return { rules: s.rules, compared: s.compared, median_ratio: s.medianRatio, p95_ratio: s.p95Ratio, over_bar_before: s.overBefore, failing: s.failing, over_alert: s.overAlert, worst: s.worst.map((w) => ({ rule: w.rule, ratio: w.ratio, baseline_ms: w.main, candidate_ms: w.candidate })) };
}

/** Slice B on one corpus: the fixed cost, every rule alone with its added time, and the corpus's rule statistics. */
function oneRuleItem(candidate: Candidate, pack: string, b: OneRuleVerdict): OneRuleItem {
  const id = `${candidate}/${pack}/one-rule/${b.corpus}`;
  const timing = (r: OneRuleTiming): RuleTiming => ({ rule: r.rule, main: r.times[0]!.main, candidate: r.times[0]!.candidate });
  const { rules: attributed, totalAddedMs } = attribute(b.rules.map(timing));
  const byRule = new Map(b.rules.map((r) => [r.rule, r]));
  const fixedTimes = b.fixed?.times.map(timeItem) ?? [];
  const phase = (name: string, side: "baseline_ms" | "candidate_ms"): number | null => fixedTimes.find((t) => t.name === name)?.[side] ?? null;
  const add = (a: number | null, c: number | null): number | null => (a === null || c === null ? null : a + c);
  const startup = { baseline_ms: add(phase("compile", "baseline_ms"), phase("evaluate", "baseline_ms")), candidate_ms: add(phase("compile", "candidate_ms"), phase("evaluate", "candidate_ms")) };
  const multiples = ruleMultiples(attributed, startup);
  const mb = (kb: number | null): number | null => (kb === null ? null : Math.round((kb / KB_PER_MB) * 10) / 10);
  const rules = attributed.map((a): RuleItem => {
    const r = byRule.get(a.rule)!;
    const time = timeItem(r.times[0]!);
    const completion = { status: completionStatus(r.completion.status), kind: r.completion.kind, reason: r.completion.reason };
    const findings = sameItem(r.findings);
    const status = worstStatus([time.status, completion.status, ...(findings === null ? [] : [findings.status]), ...(r.ruleFailure === null ? [] : (["fail"] as ItemStatus[]))]);
    return { ...a, ...multiples.get(a.rule), time, status, locator: r.locator, completion, findings, rule_failure: r.ruleFailure, wall_ms: { baseline: r.wallMs.main, candidate: r.wallMs.candidate }, rss_mb: { baseline: mb(r.rssKB.main), candidate: mb(r.rssKB.candidate) } };
  });
  const fixedCompletion = b.fixed === null ? null : { status: completionStatus(b.fixed.completion.status), kind: b.fixed.completion.kind, reason: b.fixed.completion.reason };
  const fixed = b.fixed === null || fixedCompletion === null ? null : { status: worstStatus([...fixedTimes.map((t) => t.status), fixedCompletion.status]), times: fixedTimes, locator: b.fixed.locator, completion: fixedCompletion };
  const status = b.note !== "" ? "not_compared" : worstStatus([...(fixed === null ? [] : [fixed.status]), ...rules.map((r) => r.status)]);
  return {
    id,
    corpus: b.corpus,
    status,
    reason: b.note === "" ? null : b.note,
    unmeasured: b.unmeasured,
    rule_time: ruleTimeLabel(candidate === "rr" ? RR_RULE_TIME : "compile+evaluate"),
    runs_per_side: b.runs,
    fixed,
    rules,
    total_added_ms: totalAddedMs,
    stats: ruleStatsItem(b.rules.map((r) => ({ ...r.times[0]!, name: r.rule }))),
    findings: { identical: rules.filter((r) => r.findings?.status === "pass").length, differing: rules.filter((r) => r.findings?.status === "fail").length, not_compared: rules.filter((r) => r.findings === null).length },
    hint: ruleHint({ rules: attributed, total_added_ms: totalAddedMs }),
    load: loadItem(b.load),
  };
}

/** Slice C: one file. */
function oneFileItem(candidate: Candidate, pack: string, f: OneFileVerdict): OneFileItem {
  const id = `${candidate}/${pack}/one-file/${f.label}`;
  const metrics = f.metrics.map((m) => metricItem(id, m));
  const completion = f.completion === null ? null : { status: completionStatus(f.completion.status), kind: f.completion.kind, reason: f.completion.reason, stopped_on: f.completion.stoppedOn };
  const findings = sameItem(f.findings);
  const status = f.note !== "" ? "not_compared" : worstStatus([...(completion === null ? [] : [completion.status]), ...metrics.map((m) => m.status), ...(findings === null ? [] : [findings.status]), ...(f.ruleFailures.length > 0 ? (["fail"] as ItemStatus[]) : [])]);
  return { id, label: f.label, path: f.path, bytes: f.bytes, status, reason: f.note === "" ? null : f.note, completion, metrics, findings, rule_failures: f.ruleFailures, load: { baseline: f.load.main, candidate: f.load.candidate }, sibling_busy_max: f.siblingBusyMax };
}

export interface ReproduceSettings {
  baseline: string;
  runs: number;
}

function cellItem(candidate: Candidate, pack: string, cell: CellVerdict, oneRule: OneRuleItem | undefined, reproduce: ReproduceSettings): CellItem {
  const id = `${candidate}/${pack}/${cell.corpus}`;
  const metrics = cell.metrics.map((m) => metricItem(id, m));
  const completion = cell.completion === null ? null : completionStatus(cell.completion.status);
  const findingsStatus: ItemStatus | null = cell.findings === null ? null : completionStatus(cell.findings.status);
  let status: ItemStatus;
  if (cell.blocked !== null) status = "fail";
  else if (cell.note !== "") status = "not_compared";
  else status = worstStatus([...(completion === null ? [] : [completion]), ...metrics.map((m) => m.status), ...(findingsStatus === null ? [] : [findingsStatus]), ...(cell.ruleFailures.length > 0 ? (["fail"] as ItemStatus[]) : [])]);
  let hint: string | null = null;
  if (cell.blocked !== null) hint = cell.blocked;
  else if (completion === "fail") hint = `stopped on ${cell.completion?.stoppedOn ?? "an unknown step"}`;
  else if (metrics.some((m) => m.status === "fail") && oneRule !== undefined) hint = oneRule.hint;
  else if (findingsStatus === "fail") hint = cell.findings?.samples[0] ?? null;
  else if (cell.ruleFailures.length > 0) hint = `${cell.ruleFailures[0]!.rule}: ${cell.ruleFailures[0]!.reason}`;
  return {
    id,
    candidate,
    pack,
    corpus: cell.corpus,
    status,
    reason: cell.blocked ?? (cell.note === "" ? null : cell.note),
    rules: { baseline: cell.rules.main, candidate: cell.rules.candidate },
    completion: cell.completion === null || completion === null ? null : { status: completion, kind: cell.completion.kind, reason: cell.completion.reason, stopped_on: cell.completion.stoppedOn },
    metrics,
    findings: cell.findings === null || findingsStatus === null ? null : { status: findingsStatus, reason: cell.findings.reason, baseline: counts(cell.findings.main), candidate: counts(cell.findings.candidate), lost_tp: cell.findings.lostTp, new_fp: cell.findings.newFp, samples: cell.findings.samples },
    agreement: cell.agreement === null ? null : { both: cell.agreement.both, baseline_only: cell.agreement.mainOnly, candidate_only: cell.agreement.candidateOnly, samples: cell.agreement.samples },
    rule_failures: cell.ruleFailures,
    hint,
    load: { baseline: cell.load.main, candidate: cell.load.candidate },
    sibling_busy_max: cell.siblingBusyMax,
    method_gaps: [...new Set(cell.methods.flatMap(methodGaps))],
    reproduce: `npm run bench:ship -- run --baseline ${reproduce.baseline} --packs ${pack} --corpus ${cell.corpus} --candidates ${candidate} --runs ${reproduce.runs} --out <dir>`,
    cost: cell.cost,
    caps: { memory: cell.caps.memCap, time_s: cell.caps.timeCapS },
    chunked: cell.chunked === null ? null : { size: cell.chunked.size, chunks: cell.chunked.chunks, rules_run: cell.chunked.rulesRun, rules_of: cell.chunked.rulesOf, failed_chunks: cell.chunked.failedChunks },
    progress: cell.progress === null ? null : { started: cell.progress.started, of: cell.progress.of, wall_ms: cell.progress.wallMs, rss_kb: cell.progress.rssKB },
    grouping: cell.grouping,
    fallback: cell.fallback,
  };
}

/**
 * What a pack lacks of the three slices, within the run's scope (the corpora its A cells cover; a pack without a
 * fixtures corpus has no fixtures cell to lack): an A cell not measured, a B corpus not measured or with rules not timed
 * on both sides, or fewer than C's three files measured.
 */
export function missingSlices(p: PackVerdict, cells: readonly CellItem[]): MissingSlice[] {
  if (p.noBaseline) return [];
  const absent = (cell: CellItem): boolean => (cell.reason ?? "").startsWith("no fixtures corpus");
  const missing: MissingSlice[] = cells.filter((cell) => cell.status === "not_compared" && !absent(cell)).map((cell) => ({ slice: "A", reason: `${cell.corpus}: ${cell.reason ?? "not compared"}` }));
  for (const cell of cells.filter((c) => !absent(c))) {
    const b = p.oneRule.find((o) => o.corpus === cell.corpus);
    if (b === undefined || b.note !== "") missing.push({ slice: "B", reason: `${cell.corpus}: ${b?.note ?? "not measured"}` });
    else if (b.unmeasured.length > 0) missing.push({ slice: "B", reason: `${cell.corpus}: ${b.unmeasured.length} rule${b.unmeasured.length === 1 ? "" : "s"} not timed on both sides (${b.unmeasured.slice(0, 3).join(", ")}${b.unmeasured.length > 3 ? ", …" : ""})` });
  }
  if (cells.some((cell) => cell.corpus === "real")) {
    const measured = p.oneFile.filter((f) => f.note === "");
    if (measured.length < 3) missing.push({ slice: "C", reason: `${measured.length} of 3 files measured${p.oneFile.length === 0 ? " (no files picked)" : ""}` });
  }
  return missing;
}

/** Slice B's statistics over every corpus's rules of a pack, each rule once per corpus. */
export function packRuleStats(oneRule: readonly OneRuleVerdict[]): RuleStatsItem | null {
  return oneRule.length === 0 ? null : ruleStatsItem(oneRule.flatMap((b) => b.rules.map((r) => ({ ...r.times[0]!, name: `${b.corpus}/${r.rule}` }))));
}

/** The pack table's row: each slice's status and figures, the worst A metric, the findings and the biggest contributor. */
export function packSummary(cells: readonly CellItem[], oneRule: readonly OneRuleItem[] = [], oneFile: readonly OneFileItem[] = [], incomplete: readonly MissingSlice[] = [], ruleStats: RuleStatsItem | null = null): PackSummary {
  const wallCold: PackSummary["wall_cold"] = {};
  for (const cell of cells) {
    const metric = cell.metrics.find((m) => m.metric === "wall-cold");
    wallCold[cell.corpus] = { ratio: metric?.ratio ?? null, status: metric?.status ?? cell.status, completed: cell.completion === null || cell.completion.status !== "fail", net_ratio: cell.cost?.net.find((n) => n.metric === "wall-cold")?.ratio ?? null };
  }
  const all = cells.flatMap((cell) => cell.metrics.filter((m) => m.ratio !== null && Number.isFinite(m.ratio)).map((m) => ({ cell, m })));
  const failing = all.filter(({ m }) => m.status === "fail");
  const pool = failing.length > 0 ? failing : all;
  const top = pool.reduce<(typeof pool)[number] | null>((best, x) => (best === null || x.m.ratio! > best.m.ratio! ? x : best), null);
  const worst = top === null ? null : { id: top.m.id, corpus: top.cell.corpus, metric: top.m.metric, ratio: top.m.ratio!, status: top.m.status };
  const fixtures = cells.find((cell) => cell.corpus === "fixtures")?.findings ?? null;
  const agreements = cells.map((cell) => cell.agreement).filter((a) => a !== null);
  let findings: PackSummary["findings"];
  if (fixtures === null && agreements.length === 0) findings = "not_compared";
  else if (fixtures?.status === "fail") findings = "worse";
  else {
    const fixturesSame = fixtures === null || (fixtures.baseline.tp === fixtures.candidate.tp && fixtures.baseline.fp === fixtures.candidate.fp && fixtures.baseline.fn === fixtures.candidate.fn);
    const realSame = agreements.every((a) => a.baseline_only === 0 && a.candidate_only === 0);
    findings = fixturesSame && realSame ? "same" : "no_worse";
  }
  // The hint of the cell behind the worst metric when it fails, else of any failing cell, else of a failing B or C item.
  const failingCells = cells.filter((cell) => cell.status === "fail");
  const hintCell = (top !== null && top.m.status === "fail" ? top.cell : undefined) ?? failingCells.find((cell) => cell.hint !== null);
  const failingRule = oneRule.flatMap((b) => b.rules.filter((r) => r.status === "fail").map((r) => ({ b, r })))[0];
  const failingFile = oneFile.find((f) => f.status === "fail");
  const hint =
    hintCell?.hint ??
    (failingRule !== undefined ? `B ${failingRule.b.corpus} \`${failingRule.r.rule}\`: ${failingRule.r.locator ?? failingRule.r.completion.reason}` : null) ??
    (failingFile !== undefined ? `C ${failingFile.label}: ${failingFile.metrics.find((m) => m.status === "fail")?.locator ?? failingFile.completion?.reason ?? "findings differ"}` : null);
  const sliceStatus = (slice: Slice, statuses: readonly ItemStatus[]): SliceStatus => (incomplete.some((m) => m.slice === slice) ? "missing" : worstStatus(statuses));
  const fixedWall: PackSummary["slices"]["B"]["fixed_wall"] = {};
  for (const b of oneRule) fixedWall[b.corpus] = b.fixed?.times.find((t) => t.name === "wall-cold")?.ratio ?? null;
  const loads = [...cells.flatMap((c) => [...c.load.baseline, ...c.load.candidate]), ...oneRule.map((b) => b.load.load_max), ...oneFile.flatMap((f) => [...f.load.baseline, ...f.load.candidate])].filter((l) => l !== null);
  const siblings = [...cells.map((c) => c.sibling_busy_max), ...oneRule.map((b) => b.load.sibling_busy_max), ...oneFile.map((f) => f.sibling_busy_max)].filter((s) => s !== null);
  return {
    wall_cold: wallCold,
    worst,
    findings,
    completed: cells.every((cell) => cell.completion === null || cell.completion.status !== "fail"),
    hint,
    slices: {
      A: { status: sliceStatus("A", cells.map((c) => c.status)) },
      B: {
        status: sliceStatus("B", oneRule.map((b) => b.status)),
        rules: Math.max(0, ...oneRule.map((b) => b.rules.length)),
        fixed_wall: fixedWall,
        stats: ruleStats,
        findings: { identical: oneRule.reduce((n, b) => n + b.findings.identical, 0), differing: oneRule.reduce((n, b) => n + b.findings.differing, 0) },
      },
      C: {
        status: sliceStatus("C", oneFile.map((f) => f.status)),
        files: oneFile.map((f) => {
          const ratios = f.metrics.map((m) => m.ratio).filter((r): r is number => r !== null && Number.isFinite(r));
          return { label: f.label, path: f.path, worst_ratio: ratios.length > 0 ? Math.max(...ratios) : null, metrics_status: worstStatus(f.metrics.map((m) => m.status)), findings_same: f.findings === null ? null : f.findings.status === "pass" };
        }),
      },
    },
    load: { load_max: loads.length > 0 ? Math.max(...loads) : null, sibling_busy_max: siblings.length > 0 ? Math.max(...siblings) : null },
  };
}

function packItem(candidate: Candidate, p: PackVerdict, reproduce: ReproduceSettings): PackItem {
  const oneRule = p.oneRule.map((b) => oneRuleItem(candidate, p.pack, b));
  const oneFile = p.oneFile.map((f) => oneFileItem(candidate, p.pack, f));
  const cells = p.cells.map((cell) => cellItem(candidate, p.pack, cell, oneRule.find((b) => b.corpus === cell.corpus), reproduce));
  const incomplete = missingSlices(p, cells);
  const worst = worstStatus([...cells.map((c) => c.status), ...oneRule.map((b) => b.status), ...oneFile.map((f) => f.status), ...(p.missing.length > 0 ? (["fail"] as ItemStatus[]) : [])]);
  const status: PackItem["status"] = p.noBaseline ? "no_baseline" : worst !== "fail" && incomplete.length > 0 ? "incomplete" : worst;
  const summary = packSummary(cells, oneRule, oneFile, incomplete, packRuleStats(p.oneRule));
  if (summary.hint === null && p.missing.length > 0) summary.hint = `${p.missing.length} rule${p.missing.length === 1 ? "" : "s"} cannot run: ${p.missing[0]!.rule} (${p.missing[0]!.reason})`;
  if (summary.hint === null && incomplete.length > 0) summary.hint = `incomplete: slice ${incomplete[0]!.slice} ${incomplete[0]!.reason}`;
  return { id: `${candidate}/${p.pack}`, candidate, pack: p.pack, status, counted: p.gate?.counted ?? true, gate_reason: p.gate?.reason ?? null, summary, coverage: { missing: p.missing, extra: p.extra }, cells, one_rule: oneRule, one_file: oneFile, incomplete };
}

const LABELS: Record<string, string> = { "wall-cold": "wall cold", "wall-warm": "wall warm", "cpu-cold": "CPU cold", "cpu-warm": "CPU warm", "rss-cold": "peak RSS cold", "rss-warm": "peak RSS warm" };

export function metricLabel(metric: string): string {
  return LABELS[metric] ?? metric;
}

/** What a slice B rule's time is, as the report names it: the product path's compile + evaluate, or the `.rerail` side's net wall against it. */
export function ruleTimeLabel(name: string): string {
  return name === RR_RULE_TIME ? "compile + evaluate → net wall (cold wall less the empty rule set's)" : "compile + evaluate";
}

/** The phase a run stopped in, from a completion reason ("…; stopped on <phase>"). */
function stoppedIn(reason: string): string | null {
  const phase = /stopped on (\S+)/.exec(reason)?.[1];
  return phase === undefined ? null : `stopped in ${phase}`;
}

/** Where a rule failure happened: compile errors are compile's, engine failures evaluate's. */
function failurePhase(reason: string): string {
  return reason.startsWith("compile:") ? "compile" : "evaluate";
}

/** Every item that fails (or is noisy) under one candidate, in pack order, coverage first, each slice in turn. */
function items(packs: readonly PackItem[], wanted: ItemStatus): FailureItem[] {
  const out: FailureItem[] = [];
  const bar = (1 + SLICES_TOLERANCE_PERCENT / 100).toFixed(2);
  const ratio = (r: number | null): string => (r === null || !Number.isFinite(r) ? "–" : `${r.toFixed(3)}x`);
  const metricItems = (p: PackItem, slice: Slice, corpus: Corpus | null, metrics: readonly MetricItem[], note: string, hint: string | null): void => {
    for (const m of metrics) {
      if (m.status !== wanted) continue;
      const confirmed = m.confirm === null ? "" : `; confirmation ${ratio(m.confirm.ratio)}`;
      out.push({ id: m.id, candidate: p.candidate, pack: p.pack, counted: p.counted, slice, corpus, item: m.metric, reason: `${ratio(m.ratio)} baseline (bar ${bar}x)${confirmed}${note}`, locator: m.locator, hint });
    }
  };
  for (const p of packs) {
    const push = (item: Omit<FailureItem, "candidate" | "pack" | "counted">): void => {
      out.push({ candidate: p.candidate, pack: p.pack, counted: p.counted, ...item });
    };
    if (wanted === "fail") for (const m of p.coverage.missing) push({ id: `${p.id}/coverage/${m.rule}`, slice: null, corpus: null, item: "coverage", reason: `${m.rule}: ${m.reason}`, locator: null, hint: null });
    for (const cell of p.cells) {
      if (wanted === "fail" && cell.status === "fail" && cell.reason !== null) push({ id: `${cell.id}/blocked`, slice: "A", corpus: cell.corpus, item: "completion", reason: cell.reason, locator: null, hint: null });
      if (wanted === "fail" && cell.completion?.status === "fail") push({ id: `${cell.id}/completion`, slice: "A", corpus: cell.corpus, item: "completion", reason: cell.completion.reason, locator: cell.completion.stopped_on === null ? null : `stopped in ${cell.completion.stopped_on}`, hint: null });
      if (wanted === "fail") for (const f of cell.rule_failures) push({ id: `${cell.id}/rule-failure/${f.rule}`, slice: "A", corpus: cell.corpus, item: "rule failure", reason: `${f.rule}: ${f.reason}`, locator: failurePhase(f.reason), hint: null });
      metricItems(p, "A", cell.corpus, cell.metrics, cell.chunked === null ? "" : ` (chunked: the sum of ${cell.chunked.chunks} chunk runs)`, cell.hint);
      if (wanted === "fail" && cell.findings?.status === "fail") push({ id: `${cell.id}/findings`, slice: "A", corpus: cell.corpus, item: "findings", reason: cell.findings.reason, locator: "evaluate: findings differ", hint: cell.findings.samples[0] ?? null });
    }
    for (const b of p.one_rule) {
      if (b.fixed !== null) {
        for (const t of b.fixed.times) {
          if (t.status !== wanted) continue;
          const confirmed = t.confirm === null ? "" : `; confirmation ${ratio(t.confirm.ratio)}`;
          push({ id: `${b.id}/fixed/${t.name}`, slice: "B", corpus: b.corpus, item: `fixed ${t.name}`, reason: `empty rule set ${formatMs(t.baseline_ms ?? 0)} → ${formatMs(t.candidate_ms ?? 0)}, ${ratio(t.ratio)} baseline (bar ${bar}x, floor ${RULE_FLOOR_MS} ms)${confirmed}`, locator: b.fixed.locator, hint: null });
        }
        if (wanted === "fail" && b.fixed.completion.status === "fail") push({ id: `${b.id}/fixed/completion`, slice: "B", corpus: b.corpus, item: "completion", reason: `empty rule set: ${b.fixed.completion.reason}`, locator: stoppedIn(b.fixed.completion.reason), hint: null });
      }
      for (const r of b.rules) {
        if (r.time.status === wanted) {
          const confirmed = r.time.confirm === null ? "" : `; confirmation ${formatMs(r.time.confirm.baseline_ms)} → ${formatMs(r.time.confirm.candidate_ms)} ${ratio(r.time.confirm.ratio)}`;
          push({ id: `${b.id}/${r.rule}/time`, slice: "B", corpus: b.corpus, item: "rule time", reason: `\`${r.rule}\` ${ruleTimeLabel(r.time.name)} ${formatMs(r.time.baseline_ms ?? 0)} → ${formatMs(r.time.candidate_ms ?? 0)}, ${ratio(r.time.ratio)} baseline (bar ${bar}x, floor ${RULE_FLOOR_MS} ms)${confirmed}`, locator: r.locator, hint: null });
        }
        if (wanted !== "fail") continue;
        if (r.completion.status === "fail") push({ id: `${b.id}/${r.rule}/completion`, slice: "B", corpus: b.corpus, item: "completion", reason: `\`${r.rule}\`: ${r.completion.reason}`, locator: stoppedIn(r.completion.reason), hint: null });
        if (r.findings?.status === "fail") push({ id: `${b.id}/${r.rule}/findings`, slice: "B", corpus: b.corpus, item: "findings", reason: `\`${r.rule}\`: ${r.findings.baseline_only} baseline only, ${r.findings.candidate_only} candidate only`, locator: "evaluate: findings differ", hint: r.findings.samples[0] ?? null });
        if (r.rule_failure !== null) push({ id: `${b.id}/${r.rule}/rule-failure`, slice: "B", corpus: b.corpus, item: "rule failure", reason: `\`${r.rule}\`: ${r.rule_failure}`, locator: failurePhase(r.rule_failure), hint: null });
      }
    }
    for (const f of p.one_file) {
      if (wanted === "fail" && f.completion?.status === "fail") push({ id: `${f.id}/completion`, slice: "C", corpus: "real", item: "completion", reason: `${f.path}: ${f.completion.reason}`, locator: f.completion.stopped_on === null ? null : `stopped in ${f.completion.stopped_on}`, hint: null });
      metricItems(p, "C", "real", f.metrics, ` (${f.label}: ${f.path})`, null);
      if (wanted === "fail" && f.findings?.status === "fail") push({ id: `${f.id}/findings`, slice: "C", corpus: "real", item: "findings", reason: `${f.path}: ${f.findings.baseline_only} baseline only, ${f.findings.candidate_only} candidate only`, locator: "evaluate: findings differ", hint: f.findings.samples[0] ?? null });
      if (wanted === "fail") for (const r of f.rule_failures) push({ id: `${f.id}/rule-failure/${r.rule}`, slice: "C", corpus: "real", item: "rule failure", reason: `${f.path}: ${r.rule}: ${r.reason}`, locator: failurePhase(r.reason), hint: null });
    }
  }
  return out;
}

export interface RunMeta {
  startedAt: string | null;
  finishedAt: string;
  invokedBy: string | null;
  command: string | null;
  harnessSha: string;
  confirmRuns: number;
  ruleRuns: number;
  fileRuns: number;
}

export function buildReport(verdict: Verdict, meta: RunMeta, waits: WaitsSection | null = null): Report {
  const reproduce: ReproduceSettings = { baseline: verdict.baseline.ref ?? verdict.baseline.sha, runs: verdict.runs };
  const partial = verdict.partial;
  const candidates: CandidateItem[] = verdict.candidates.map((c) => {
    const packs = c.packs.map((p) => packItem(c.candidate, p, reproduce));
    const failures = items(packs, "fail");
    const noisy = items(packs, "noisy");
    const statusOf = (subset: readonly PackItem[]): VerdictStatus => (items(subset, "fail").length > 0 ? "FAIL" : subset.some((p) => p.incomplete.length > 0) ? "INCOMPLETE" : partial.length > 0 ? "PARTIAL" : "PASS");
    const counted = packs.filter((p) => p.counted);
    return {
      candidate: c.candidate,
      description: c.description,
      status: statusOf(packs),
      counted_status: counted.length === 0 ? null : statusOf(counted),
      counts: {
        packs: packs.filter((p) => p.status !== "no_baseline").length,
        failing_packs: packs.filter((p) => p.status === "fail").length,
        noisy_packs: packs.filter((p) => p.status === "noisy").length,
        incomplete_packs: packs.filter((p) => p.incomplete.length > 0).length,
        reported_packs: packs.filter((p) => !p.counted).length,
        failing_items: failures.length,
        noisy_items: noisy.length,
      },
      failures,
      noisy,
      packs,
    };
  });
  // A gated candidate counts by its counted packs only (none counted: it cannot fail the run); a narrowed run is never PASS.
  const gated = candidates.filter((c) => verdict.gate.includes(c.candidate));
  const statuses: VerdictStatus[] = [...gated.flatMap((c) => (c.counted_status === null ? [] : [c.counted_status])), ...(waits === null ? [] : [waits.status]), ...(partial.length > 0 && candidates.length > 0 ? (["PARTIAL"] as const) : [])];
  const overallStatus: VerdictStatus = (["FAIL", "INCOMPLETE", "PARTIAL"] as const).find((s) => statuses.includes(s)) ?? "PASS";
  return {
    schema: SCHEMA,
    run: { started_at: meta.startedAt, finished_at: meta.finishedAt, invoked_by: meta.invokedBy, command: meta.command, harness_sha: meta.harnessSha },
    machines: verdict.machines,
    baseline: { ref: verdict.baseline.ref ?? null, sha: verdict.baseline.sha },
    candidate: { ref: verdict.candidate.ref ?? null, sha: verdict.candidate.sha },
    inputs: verdict.corpora,
    bar: {
      max_ratio: 1 + SLICES_TOLERANCE_PERCENT / 100,
      runs_per_side: verdict.runs,
      confirm_runs: meta.confirmRuns,
      rule_runs: meta.ruleRuns,
      file_runs: meta.fileRuns,
      rule_floor_ms: RULE_FLOOR_MS,
      rule_alert_ratio: RULE_ALERT_RATIO,
      caps: { memory: verdict.machine.memCap, time_s: verdict.machine.timeCapS },
      pinned_cpus: verdict.machine.cpus,
      threads: verdict.machine.threads,
      methods: verdict.machine.methods.map((m) => ({ ...m, gaps: methodGaps(m.method) })),
    },
    verdict: { status: overallStatus, gate: verdict.gate, partial: candidates.length > 0 ? [...partial] : [], candidates: Object.fromEntries(candidates.map((c) => [c.candidate, { status: c.status, counted_status: c.counted_status, failing_packs: c.counts.failing_packs, failing_items: c.counts.failing_items }])), waits: waits?.status ?? null },
    candidates,
    waits,
    scope: {
      measured: [
        ...(candidates.length > 0
          ? [
              "every pack the baseline ships in three slices: A, all of its rules on its upstream fixtures and one pinned real-code corpus (wall, CPU and peak RSS, cold and warm; findings; completion; coverage); B, the empty rule set and each rule alone on both corpora (compile + evaluate per rule, the `.rerail` side's net wall against it, the fixed cost's wall and phases; findings per rule); C, all of its rules on three files of the real-code corpus (wall, CPU and peak RSS, cold and warm; findings)",
            ]
          : []),
        ...(waits === null ? [] : [`user waits through hooks, daemon and CLI: ${[...new Set(waits.items.map((i) => i.wait))].join(", ")}`]),
      ],
      not_measured: [
        ...(candidates.length > 0 ? [] : ["pack timing (§5.1): no pack runs in this directory"]),
        ...(waits === null ? ["user waits through hooks, daemon and CLI (W1–W8)"] : waits.not_measured.map((n) => `${n.id}: ${n.reason}`)),
        "a repository's whole enabled rule set across packs, and the rule work budget",
        ...(waits === null ? [] : ["user waits on macOS and Windows: waits measure on Linux only"]),
      ],
      known_gaps: KNOWN_GAPS,
    },
  };
}
