// The ship / no-ship report, report.json (schema ship-no-ship/report/v1), built from what shipbar.ts measured. Every
// pack, corpus and metric keeps its raw runs, ratio and status under a stable id, so runs compare across nights; each
// level names its biggest contributor one level down (a pack its worst metric and the rule behind it), so a reader can
// stop at the pack table. A metric that fails at the median and passes its confirmation runs is noisy: reported, not
// counted. Pure functions; report.ts renders the Markdown.

import type { Candidate, CellVerdict, Corpus, MeasuredMetric, PackVerdict, RuleTiming, Verdict } from "./shipbar.js";
import { median, spread, TOLERANCE_PERCENT } from "./verdict.js";

export const SCHEMA = "ship-no-ship/report/v1";

/** noisy: failed at the median and passed its confirmation runs; reported and not counted. */
export type ItemStatus = "pass" | "fail" | "noisy" | "not_compared";

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
}

export interface RuleItem {
  rule: string;
  baseline_ms: number | null;
  candidate_ms: number | null;
  added_ms: number | null;
  /** added_ms over the cell's total added time, when that total is positive. */
  share: number | null;
}

export interface DrillItem {
  /** evaluate_ms: the product path's in-process evaluation; wall_ms: process wall less the empty rule set's. */
  measure: "evaluate_ms" | "wall_ms";
  reps: number;
  startup: { baseline_ms: number | null; candidate_ms: number | null };
  total_added_ms: number;
  rules: RuleItem[];
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
  drill: DrillItem | null;
  /** The biggest contributor one level down: the rule behind a slowdown, where a run stopped, a differing finding. */
  hint: string | null;
  load: { baseline: number[]; candidate: number[] };
  sibling_busy_max: number;
  reproduce: string;
}

export interface PackSummary {
  /** Each corpus's wall-cold ratio and status: the pack table's two columns. */
  wall_cold: Partial<Record<Corpus, { ratio: number | null; status: ItemStatus; completed: boolean }>>;
  /** The failing metric with the highest ratio, else the highest ratio of any metric. */
  worst: { id: string; corpus: Corpus; metric: string; ratio: number; status: ItemStatus } | null;
  findings: "same" | "no_worse" | "worse" | "not_compared";
  completed: boolean;
  hint: string | null;
}

export interface PackItem {
  id: string;
  candidate: Candidate;
  pack: string;
  status: ItemStatus | "no_baseline";
  summary: PackSummary;
  coverage: { missing: { rule: string; reason: string }[]; extra: string[] };
  cells: CellItem[];
}

export interface FailureItem {
  id: string;
  candidate: Candidate;
  pack: string;
  corpus: Corpus | null;
  item: string;
  reason: string;
  hint: string | null;
}

export interface CandidateItem {
  candidate: Candidate;
  description: string;
  status: "PASS" | "FAIL" | "INCOMPLETE";
  counts: { packs: number; failing_packs: number; noisy_packs: number; failing_items: number; noisy_items: number };
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
  bar: { max_ratio: number; runs_per_side: number; confirm_runs: number; drill_reps: number; caps: { memory: string[]; time_s: number[] }; pinned_cpus: string[]; threads: number[] };
  verdict: { status: "PASS" | "FAIL" | "INCOMPLETE"; gate: Candidate[]; candidates: Partial<Record<Candidate, { status: CandidateItem["status"]; failing_packs: number; failing_items: number }>> };
  candidates: CandidateItem[];
  scope: { measured: string[]; not_measured: string[] };
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
 * time of the other rules, and at least MIN_SHARE of the cell's total. At most MAX_NAMED rules are named; a slowdown
 * no rule stands out in is reported as spread over every rule.
 */
export const STANDS_OUT = 3;
export const MIN_SHARE = 0.2;
export const MAX_NAMED = 2;

/** Each rule's added time, largest first (rules a side could not run last), and the cell's total added time. */
export function attribute(rules: readonly RuleTiming[]): { rules: RuleItem[]; totalAddedMs: number } {
  const items = rules.map((r): RuleItem => ({ rule: r.rule, baseline_ms: r.main, candidate_ms: r.candidate, added_ms: r.main === null || r.candidate === null ? null : r.candidate - r.main, share: null }));
  const totalAddedMs = items.reduce((sum, r) => sum + (r.added_ms ?? 0), 0);
  for (const r of items) r.share = r.added_ms !== null && totalAddedMs > 0 ? r.added_ms / totalAddedMs : null;
  items.sort((a, b) => (b.added_ms ?? Number.NEGATIVE_INFINITY) - (a.added_ms ?? Number.NEGATIVE_INFINITY) || (a.rule < b.rule ? -1 : 1));
  return { rules: items, totalAddedMs };
}

/** The rule-level hint of a slower cell: the rules that stand out, or how much each rule moved when none does. */
export function drillHint(drill: DrillItem): string | null {
  const timed = drill.rules.filter((r) => r.added_ms !== null);
  if (timed.length === 0) return null;
  const total = drill.total_added_ms;
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
  };
}

function drillItem(cell: CellVerdict): DrillItem | null {
  if (cell.drill === null) return null;
  const { rules, totalAddedMs } = attribute(cell.drill.rules);
  return { measure: cell.drill.measure, reps: cell.drill.reps, startup: { baseline_ms: cell.drill.startup.main, candidate_ms: cell.drill.startup.candidate }, total_added_ms: totalAddedMs, rules };
}

function counts(c: { tp: number; fp: number; fn: number; explainedFp: number; explainedFn: number }): Counts {
  return { tp: c.tp, fp: c.fp, fn: c.fn, explained_fp: c.explainedFp, explained_fn: c.explainedFn };
}

export interface ReproduceSettings {
  baseline: string;
  runs: number;
}

function cellItem(candidate: Candidate, pack: string, cell: CellVerdict, reproduce: ReproduceSettings): CellItem {
  const id = `${candidate}/${pack}/${cell.corpus}`;
  const metrics = cell.metrics.map((m) => metricItem(id, m));
  const drill = drillItem(cell);
  const completionStatus: ItemStatus | null = cell.completion === null ? null : cell.completion.status === "fail" ? "fail" : cell.completion.status === "pass" ? "pass" : "not_compared";
  const findingsStatus: ItemStatus | null = cell.findings === null ? null : cell.findings.status === "fail" ? "fail" : cell.findings.status === "pass" ? "pass" : "not_compared";
  let status: ItemStatus;
  if (cell.blocked !== null) status = "fail";
  else if (cell.note !== "") status = "not_compared";
  else status = worstStatus([...(completionStatus === null ? [] : [completionStatus]), ...metrics.map((m) => m.status), ...(findingsStatus === null ? [] : [findingsStatus]), ...(cell.ruleFailures.length > 0 ? (["fail"] as ItemStatus[]) : [])]);
  let hint: string | null = null;
  if (cell.blocked !== null) hint = cell.blocked;
  else if (completionStatus === "fail") hint = `stopped on ${cell.completion?.stoppedOn ?? "an unknown step"}`;
  else if (metrics.some((m) => m.status === "fail") && drill !== null) hint = drillHint(drill);
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
    completion: cell.completion === null || completionStatus === null ? null : { status: completionStatus, kind: cell.completion.kind, reason: cell.completion.reason, stopped_on: cell.completion.stoppedOn },
    metrics,
    findings: cell.findings === null || findingsStatus === null ? null : { status: findingsStatus, reason: cell.findings.reason, baseline: counts(cell.findings.main), candidate: counts(cell.findings.candidate), lost_tp: cell.findings.lostTp, new_fp: cell.findings.newFp, samples: cell.findings.samples },
    agreement: cell.agreement === null ? null : { both: cell.agreement.both, baseline_only: cell.agreement.mainOnly, candidate_only: cell.agreement.candidateOnly, samples: cell.agreement.samples },
    rule_failures: cell.ruleFailures,
    drill,
    hint,
    load: { baseline: cell.load.main, candidate: cell.load.candidate },
    sibling_busy_max: cell.siblingBusyMax,
    reproduce: `npm run bench:ship -- run --baseline ${reproduce.baseline} --packs ${pack} --corpus ${cell.corpus} --candidates ${candidate} --runs ${reproduce.runs} --out <dir>`,
  };
}

/** The pack table's row: each corpus's wall-cold ratio, the worst metric, the findings and the biggest contributor. */
export function packSummary(cells: readonly CellItem[]): PackSummary {
  const wallCold: PackSummary["wall_cold"] = {};
  for (const cell of cells) {
    const metric = cell.metrics.find((m) => m.metric === "wall-cold");
    wallCold[cell.corpus] = { ratio: metric?.ratio ?? null, status: metric?.status ?? cell.status, completed: cell.completion === null || cell.completion.status !== "fail" };
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
  // The hint of the cell behind the worst metric when it fails, else of any failing cell.
  const failingCells = cells.filter((cell) => cell.status === "fail");
  const hintCell = (top !== null && top.m.status === "fail" ? top.cell : undefined) ?? failingCells.find((cell) => cell.hint !== null);
  return { wall_cold: wallCold, worst, findings, completed: cells.every((cell) => cell.completion === null || cell.completion.status !== "fail"), hint: hintCell?.hint ?? null };
}

function packItem(candidate: Candidate, p: PackVerdict, reproduce: ReproduceSettings): PackItem {
  const cells = p.cells.map((cell) => cellItem(candidate, p.pack, cell, reproduce));
  const status: PackItem["status"] = p.noBaseline ? "no_baseline" : worstStatus([...cells.map((c) => c.status), ...(p.missing.length > 0 ? (["fail"] as ItemStatus[]) : [])]);
  const summary = packSummary(cells);
  if (summary.hint === null && p.missing.length > 0) summary.hint = `${p.missing.length} rule${p.missing.length === 1 ? "" : "s"} cannot run: ${p.missing[0]!.rule} (${p.missing[0]!.reason})`;
  return { id: `${candidate}/${p.pack}`, candidate, pack: p.pack, status, summary, coverage: { missing: p.missing, extra: p.extra }, cells };
}

const LABELS: Record<string, string> = { "wall-cold": "wall cold", "wall-warm": "wall warm", "cpu-cold": "CPU cold", "cpu-warm": "CPU warm", "rss-cold": "peak RSS cold", "rss-warm": "peak RSS warm" };

export function metricLabel(metric: string): string {
  return LABELS[metric] ?? metric;
}

/** Every item that fails (or is noisy) under one candidate, in pack order, coverage first. */
function items(packs: readonly PackItem[], wanted: ItemStatus): FailureItem[] {
  const out: FailureItem[] = [];
  for (const p of packs) {
    if (wanted === "fail") for (const m of p.coverage.missing) out.push({ id: `${p.id}/coverage/${m.rule}`, candidate: p.candidate, pack: p.pack, corpus: null, item: "coverage", reason: `${m.rule}: ${m.reason}`, hint: null });
    for (const cell of p.cells) {
      if (wanted === "fail" && cell.status === "fail" && cell.reason !== null) out.push({ id: `${cell.id}/blocked`, candidate: p.candidate, pack: p.pack, corpus: cell.corpus, item: "completion", reason: cell.reason, hint: null });
      if (wanted === "fail" && cell.completion?.status === "fail") out.push({ id: `${cell.id}/completion`, candidate: p.candidate, pack: p.pack, corpus: cell.corpus, item: "completion", reason: cell.completion.reason, hint: cell.completion.stopped_on === null ? null : `stopped on ${cell.completion.stopped_on}` });
      if (wanted === "fail") for (const f of cell.rule_failures) out.push({ id: `${cell.id}/rule-failure/${f.rule}`, candidate: p.candidate, pack: p.pack, corpus: cell.corpus, item: "rule failure", reason: `${f.rule}: ${f.reason}`, hint: null });
      for (const m of cell.metrics) {
        if (m.status !== wanted) continue;
        const bar = 1 + TOLERANCE_PERCENT / 100;
        const confirmed = m.confirm === null ? "" : `; confirmation ${m.confirm.ratio === null ? "–" : `${m.confirm.ratio.toFixed(3)}x`}`;
        out.push({ id: m.id, candidate: p.candidate, pack: p.pack, corpus: cell.corpus, item: m.metric, reason: `${m.ratio!.toFixed(3)}x baseline (bar ${bar.toFixed(2)}x)${confirmed}`, hint: cell.hint });
      }
      if (wanted === "fail" && cell.findings?.status === "fail") out.push({ id: `${cell.id}/findings`, candidate: p.candidate, pack: p.pack, corpus: cell.corpus, item: "findings", reason: cell.findings.reason, hint: cell.findings.samples[0] ?? null });
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
  drillReps: number;
}

export function buildReport(verdict: Verdict, meta: RunMeta): Report {
  const reproduce: ReproduceSettings = { baseline: verdict.baseline.ref ?? verdict.baseline.sha, runs: verdict.runs };
  const candidates: CandidateItem[] = verdict.candidates.map((c) => {
    const packs = c.packs.map((p) => packItem(c.candidate, p, reproduce));
    const failures = items(packs, "fail");
    const noisy = items(packs, "noisy");
    const incomplete = packs.some((p) => p.cells.some((cell) => cell.status === "not_compared" && !(cell.reason ?? "").startsWith("no fixtures corpus")));
    const status = failures.length > 0 ? "FAIL" : incomplete ? "INCOMPLETE" : "PASS";
    return {
      candidate: c.candidate,
      description: c.description,
      status,
      counts: { packs: packs.filter((p) => p.status !== "no_baseline").length, failing_packs: packs.filter((p) => p.status === "fail").length, noisy_packs: packs.filter((p) => p.status === "noisy").length, failing_items: failures.length, noisy_items: noisy.length },
      failures,
      noisy,
      packs,
    };
  });
  const gated = candidates.filter((c) => verdict.gate.includes(c.candidate));
  const overallStatus = gated.some((c) => c.status === "FAIL") ? "FAIL" : gated.some((c) => c.status === "INCOMPLETE") ? "INCOMPLETE" : "PASS";
  return {
    schema: SCHEMA,
    run: { started_at: meta.startedAt, finished_at: meta.finishedAt, invoked_by: meta.invokedBy, command: meta.command, harness_sha: meta.harnessSha },
    machines: verdict.machines,
    baseline: { ref: verdict.baseline.ref ?? null, sha: verdict.baseline.sha },
    candidate: { ref: verdict.candidate.ref ?? null, sha: verdict.candidate.sha },
    inputs: verdict.corpora,
    bar: { max_ratio: 1 + TOLERANCE_PERCENT / 100, runs_per_side: verdict.runs, confirm_runs: meta.confirmRuns, drill_reps: meta.drillReps, caps: { memory: verdict.machine.memCap, time_s: verdict.machine.timeCapS }, pinned_cpus: verdict.machine.cpus, threads: verdict.machine.threads },
    verdict: { status: overallStatus, gate: verdict.gate, candidates: Object.fromEntries(candidates.map((c) => [c.candidate, { status: c.status, failing_packs: c.counts.failing_packs, failing_items: c.counts.failing_items }])) },
    candidates,
    scope: {
      measured: ["every pack the baseline ships, all of its rules enabled together, on its upstream fixtures and one pinned real-code corpus: wall, CPU and peak RSS, cold and warm; findings; completion; coverage"],
      not_measured: ["user waits through hooks, daemon and CLI (W1–W8)", "a repository's whole enabled rule set across packs, and the rule work budget", "macOS and Windows"],
    },
  };
}
