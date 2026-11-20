// The waits a user experiences (PRD §5.3, W1–W8), as the ship / no-ship report's `waits` section: what waits.ts
// measured through the product's own entry points, compared at the same bar as the packs. Pure functions over the run
// rows waits.ts writes (<out>/waits/runs.jsonl); report.ts renders the section.
//
// Two kinds of wait:
//   distribution  hundreds of events per side (W1, W2, W8), pooled over the side's replays. p50 and p95 fail only when
//                 the 95% bootstrap confidence interval of the candidate / baseline ratio lies entirely above the bar;
//                 p99 and the maximum are reported. A rate (late answers, "not answering") fails when the candidate's
//                 is higher than the baseline's (§6.3).
//   single        one figure per run (W3–W7): each side's median of its runs, compared as the packs' metrics are
//                 (verdict.ts compareMetric).
// A scenario failing in its first runs is run again (`confirm` rows); a metric failing again is confirmed, one passing
// is noisy (reported, not counted). Absolute limits (§6.3) apply to the candidate's figure whatever the baseline's; a
// limit left unset is reported as such.

import { type ItemStatus, worstStatus } from "./report-model.js";
import { compareMetric, spread, TOLERANCE_PERCENT } from "./verdict.js";
import { FILE_EDITS, type Shape, SHAPES, type Step } from "./waits-steps.js";

export const WAITS = ["W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"] as const;
export type WaitId = (typeof WAITS)[number];
export type WaitSide = "baseline" | "candidate";

/** A metric of a scenario: a quantile or the maximum of a distribution, a rate of flagged events, or one wall time. */
export type MetricSpec =
  | { metric: string; sample: string; kind: "quantile"; q: number; gated: boolean }
  | { metric: string; sample: string; kind: "max" }
  | { metric: string; sample: string; kind: "rate" }
  | { metric: string; sample: string; kind: "median" };

export interface ScenarioSpec {
  wait: WaitId;
  scenario: string;
  title: string;
  /** The product entry point measured, as a user meets it. */
  entry: string;
  store: string;
  kind: "distribution" | "single";
  metrics: MetricSpec[];
  /** The limit names (waits.json `limits`) that bound this scenario's metrics: limit name → metric. */
  limits: Record<string, string>;
  /** A step scenario's shape and step (waits-steps.ts). */
  shape?: Shape;
  step?: Step;
}

const quantiles = (sample: string, extra: MetricSpec[] = []): MetricSpec[] => [
  { metric: "p50", sample, kind: "quantile", q: 0.5, gated: true },
  { metric: "p95", sample, kind: "quantile", q: 0.95, gated: true },
  { metric: "p99", sample, kind: "quantile", q: 0.99, gated: false },
  { metric: "max", sample, kind: "max" },
  ...extra,
];

const single = (wait: WaitId, scenario: string, title: string, entry: string, store: string, sample = "wall_ms", limits: Record<string, string> = {}): ScenarioSpec => ({
  wait, scenario, title, entry, store, kind: "single", metrics: [{ metric: "median", sample, kind: "median" }], limits,
});

/** Every scenario waits.ts can measure, in report order. Ids are stable: wait/<W>/<scenario>. */
export const SCENARIOS: readonly ScenarioSpec[] = [
  {
    wait: "W1", scenario: "replay", title: "Check after an edit", entry: "Claude Code PostToolUse hook → daemon, a replayed session", store: "warm", kind: "distribution",
    metrics: quantiles("verdict_ms", [{ metric: "late_rate", sample: "late", kind: "rate" }]),
    limits: { "W1.p95_ms": "p95" },
  },
  { wait: "W2", scenario: "quiet-hook", title: "A hook call with nothing to report", entry: "Claude Code hooks of the replayed session that answered {}", store: "warm", kind: "distribution", metrics: quantiles("hook_ms"), limits: {} },
  single("W3", "first-session", "First session in a repository", "SessionStart hook (autostarts the daemon) → first edit's verdict", "first ever", "first_verdict_ms"),
  single("W4", "upgrade", "First session after an upgrade", "SessionStart hook → first edit's verdict, on a home and store the baseline wrote", "previous release's", "first_verdict_ms"),
  single("W5", "scan-first", "Repository scan, first ever", "rerail scan --scope repository --wait", "first ever", "wall_ms", { "W5.scan_wait_ms": "median" }),
  single("W5", "scan-warm", "Repository scan, warm", "rerail scan --scope repository --wait", "warm", "wall_ms", { "W5.scan_wait_ms": "median" }),
  single("W5", "scan-switch", "Repository scan after a branch switch", "git checkout, then rerail scan --scope repository --wait", "warm, other branch", "wall_ms", { "W5.scan_wait_ms": "median" }),
  single("W6", "daemon-start-first", "Daemon start, first ever", "rerail daemon start (returns once ready)", "first ever"),
  single("W6", "daemon-start-warm", "Daemon start, warm", "rerail daemon start (returns once ready)", "warm"),
  single("W6", "autostart", "Session start that starts the daemon", "SessionStart hook with no daemon running", "first ever"),
  single("W6", "session-open", "Session start, packs loaded and compiled", "SessionStart hook, daemon running", "warm"),
  single("W7", "status", "CLI start-up: status", "rerail status", "warm"),
  single("W7", "packs-list", "CLI start-up: packs", "rerail packs list", "warm"),
  single("W7", "rules-show", "CLI start-up: rules", "rerail rules show <rule>", "warm"),
  {
    wait: "W8", scenario: "status-during-audit", title: "Status while an audit runs", entry: "rerail status during a repository scan", store: "warm", kind: "distribution",
    metrics: quantiles("status_ms", [{ metric: "not_answering_rate", sample: "not_answering", kind: "rate" }]), limits: {},
  },
  { wait: "W8", scenario: "hook-during-audit", title: "Hook while an audit runs", entry: "Claude Code PreToolUse and PostToolUse hooks during a repository scan", store: "warm", kind: "distribution", metrics: quantiles("hook_ms"), limits: {} },
  ...(Object.keys(SHAPES) as Shape[]).flatMap((shape) =>
    (SHAPES[shape].steps as readonly Step[]).flatMap((step): ScenarioSpec[] => {
      const store = step === "cold" ? "first ever" : "warm";
      const scanned: ScenarioSpec = {
        ...single("W5", `steps-${shape}-${step}`, `Scan after the ${step} step (${shape})`, `the ${step} step, then rerail scan --scope repository --wait`, store, "wall_ms", { "W5.scan_wait_ms": "median" }),
        shape, step,
      };
      if (!FILE_EDITS.includes(step)) return [scanned];
      return [{ ...single("W1", `steps-${shape}-${step}`, `Verdict on the ${step} edit (${shape})`, "the edit as a Write: PreToolUse, the file, PostToolUse → daemon", store, "verdict_ms"), shape, step }, scanned];
    }),
  ),
];

export function scenarioId(spec: { wait: string; scenario: string }): string {
  return `wait/${spec.wait}/${spec.scenario}`;
}

// ---------------------------------------------------------------- rows

/** One run of one side of one scenario: a single wait's figure, or one replay's events. */
export interface WaitRun {
  wait: WaitId;
  scenario: string;
  side: WaitSide;
  phase: "runs" | "confirm";
  rep: number;
  ok: boolean;
  reason: string | null;
  /** Sample name → values: one value for a single wait; per event for a distribution (rates as 0 / 1). */
  samples: Record<string, number[]>;
  /** Observations kept with the run (a scan inside --wait's 10 s, the events replayed, …). */
  notes: Record<string, number | string | boolean | null>;
  load: number;
  pinnedBusy: number;
  siblingBusy: number;
  /** Whether the OS page cache was dropped before the run; null where the run does not start cold. */
  cacheDropped: boolean | null;
  started: string;
  host: string;
  cpus: string;
}

// ---------------------------------------------------------------- statistics

/** The q-quantile of `values` by linear interpolation between order statistics (R's type 7). */
export function quantile(values: readonly number[], q: number): number {
  if (values.length === 0) throw new Error("quantile of no values");
  const sorted = [...values].sort((a, b) => a - b);
  return sortedQuantile(sorted, q);
}

function sortedQuantile(sorted: readonly number[], q: number): number {
  const position = (sorted.length - 1) * q;
  const low = Math.floor(position);
  const high = Math.ceil(position);
  return sorted[low]! + (sorted[high]! - sorted[low]!) * (position - low);
}

/** A small seeded generator (mulberry32), so a report's intervals are the same each time it is built. */
export function seededRandom(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function seedOf(text: string): number {
  let hash = 2166136261;
  for (let i = 0; i < text.length; i += 1) hash = Math.imul(hash ^ text.charCodeAt(i), 16777619);
  return hash >>> 0;
}

export const BOOTSTRAP_RESAMPLES = 2000;

/**
 * The candidate / baseline ratio of the q-quantile, with its percentile-bootstrap 95% confidence interval: each side
 * resampled with replacement on its own, `resamples` times.
 */
export function bootstrapRatio(baseline: readonly number[], candidate: readonly number[], q: number, seed: string, resamples = BOOTSTRAP_RESAMPLES): { ratio: number; lo: number; hi: number } {
  const ratioOf = (b: number, c: number): number => (b === 0 ? (c === 0 ? 1 : Number.POSITIVE_INFINITY) : c / b);
  const random = seededRandom(seedOf(seed));
  const resample = (values: readonly number[]): number[] => {
    const out = new Array<number>(values.length);
    for (let i = 0; i < values.length; i += 1) out[i] = values[Math.floor(random() * values.length)]!;
    return out.sort((a, b) => a - b);
  };
  const ratios: number[] = [];
  for (let i = 0; i < resamples; i += 1) ratios.push(ratioOf(sortedQuantile(resample(baseline), q), sortedQuantile(resample(candidate), q)));
  ratios.sort((a, b) => a - b);
  return { ratio: ratioOf(quantile(baseline, q), quantile(candidate, q)), lo: sortedQuantile(ratios, 0.025), hi: sortedQuantile(ratios, 0.975) };
}

/** A sample's rate: flagged events (1) over all events. */
export function rate(values: readonly number[]): number {
  return values.length === 0 ? 0 : values.filter((v) => v !== 0).length / values.length;
}

/** One metric compared on one set of runs (first runs or confirmation runs). */
export interface Comparison {
  baseline: number | null;
  candidate: number | null;
  ratio: number | null;
  ci: { lo: number; hi: number } | null;
  /** Events (distribution) or runs (single) behind each side's figure. */
  n: { baseline: number; candidate: number };
  spread_pct: { baseline: number | null; candidate: number | null };
  status: "pass" | "fail" | "not_compared";
  /** The verdict could go either way: the bar lies inside the confidence interval, or within the runs' spread. */
  within_spread: boolean;
}

const BAR = 1 + TOLERANCE_PERCENT / 100;

/** `spec` on each side's pooled samples. Ungated metrics (p99, max) get a status from their point ratio, never counted. */
export function compare(spec: MetricSpec, baseline: readonly number[], candidate: readonly number[], seed: string): Comparison {
  const n = { baseline: baseline.length, candidate: candidate.length };
  const none: Comparison = { baseline: null, candidate: null, ratio: null, ci: null, n, spread_pct: { baseline: null, candidate: null }, status: "not_compared", within_spread: false };
  if (spec.kind === "rate") {
    if (baseline.length === 0 || candidate.length === 0) return { ...none, baseline: baseline.length > 0 ? rate(baseline) : null, candidate: candidate.length > 0 ? rate(candidate) : null };
    const b = rate(baseline);
    const c = rate(candidate);
    return { ...none, baseline: b, candidate: c, ratio: b === 0 ? (c === 0 ? 1 : Number.POSITIVE_INFINITY) : c / b, status: c > b ? "fail" : "pass" };
  }
  if (spec.kind === "median") {
    const verdict = compareMetric(spec.metric, baseline, candidate);
    const pct = (s: number | null): number | null => (s === null ? null : Math.round(s * 1000) / 10);
    return {
      baseline: verdict.main, candidate: verdict.candidate, ratio: verdict.ratio, ci: null, n,
      spread_pct: { baseline: pct(verdict.spreadMain), candidate: pct(verdict.spreadCandidate) },
      status: verdict.status === "not-compared" ? "not_compared" : verdict.status, within_spread: verdict.withinSpread,
    };
  }
  if (baseline.length === 0 || candidate.length === 0) return none;
  if (spec.kind === "max") {
    const b = Math.max(...baseline);
    const c = Math.max(...candidate);
    const ratio = b === 0 ? (c === 0 ? 1 : Number.POSITIVE_INFINITY) : c / b;
    return { ...none, baseline: b, candidate: c, ratio, status: ratio > BAR ? "fail" : "pass" };
  }
  const { ratio, lo, hi } = bootstrapRatio(baseline, candidate, spec.q, `${seed}/${spec.metric}`);
  // Gated quantiles fail only on the interval; p99 is reported by its point ratio.
  const failing = spec.gated ? lo > BAR : ratio > BAR;
  return {
    ...none, baseline: quantile(baseline, spec.q), candidate: quantile(candidate, spec.q), ratio, ci: { lo, hi },
    spread_pct: { baseline: Math.round(spread(baseline) * 1000) / 10, candidate: Math.round(spread(candidate) * 1000) / 10 },
    status: failing ? "fail" : "pass", within_spread: lo <= BAR && BAR <= hi,
  };
}

// ---------------------------------------------------------------- the section

export interface WaitMetricItem {
  id: string;
  metric: string;
  unit: "ms" | "rate";
  /** Counted in the scenario's status: gated quantiles, medians and rates; p99 and the maximum are reported only. */
  gated: boolean;
  first: Comparison;
  confirm: Comparison | null;
  /** An absolute limit on the candidate's figure (§6.3); `value` null when it is not set yet. */
  limit: { name: string; value: number | null; baseline_within: boolean | null; candidate_within: boolean | null } | null;
  status: ItemStatus;
}

export interface WaitItem {
  id: string;
  wait: WaitId;
  scenario: string;
  title: string;
  entry: string;
  store: string;
  kind: ScenarioSpec["kind"];
  /** A step scenario's shape and step (waits-steps.ts); null for the other waits. */
  shape: Shape | null;
  step: Step | null;
  /** Counted in the waits verdict; a scenario the configuration reports only (W4 by default) is not. */
  gated: boolean;
  status: ItemStatus;
  reason: string | null;
  metrics: WaitMetricItem[];
  runs: { baseline: number; candidate: number; confirm: number; failed: { side: WaitSide; rep: number; reason: string }[] };
  /** The scenario's observations, per side (scans finished inside --wait, events replayed, …). */
  notes: Record<WaitSide, Record<string, (number | string | boolean | null)[]>>;
  load: { baseline: number[]; candidate: number[] };
  sibling_busy_max: number;
  cache_dropped: boolean | null;
  hint: string | null;
  reproduce: string;
}

export interface WaitsSection {
  status: "PASS" | "FAIL" | "INCOMPLETE";
  counts: { scenarios: number; failing: number; noisy: number };
  /** Limit name → value; null: not set yet (§12). */
  limits: Record<string, number | null>;
  inputs: { sessions: Record<string, { edits: number; sha256: string; source: string }>; repos: Record<string, { corpus: string; packs: string[]; files: number; sha256: string }> };
  method: string[];
  items: WaitItem[];
  not_measured: { id: string; reason: string }[];
}

export interface WaitsConfig {
  limits: Record<string, number | null>;
  /** Waits reported but not counted in the verdict (PRD §12.6 leaves W4 open). */
  reportOnly: string[];
  inputs: WaitsSection["inputs"];
  method: string[];
  /** Scenario ids requested; those without runs are listed as not measured with `skipped`'s reason. */
  requested: string[];
  skipped: Record<string, string>;
  reproduce: (spec: ScenarioSpec) => string;
}

function pooled(runs: readonly WaitRun[], side: WaitSide, sample: string): number[] {
  return runs.filter((r) => r.side === side && r.ok).flatMap((r) => r.samples[sample] ?? []);
}

function metricItem(id: string, spec: MetricSpec, first: readonly WaitRun[], confirm: readonly WaitRun[], limits: Record<string, number | null>, limitName: string | undefined): WaitMetricItem {
  const firstComparison = compare(spec, pooled(first, "baseline", spec.sample), pooled(first, "candidate", spec.sample), id);
  const confirmed = confirm.length > 0 ? compare(spec, pooled(confirm, "baseline", spec.sample), pooled(confirm, "candidate", spec.sample), `${id}/confirm`) : null;
  const gated = spec.kind === "median" || spec.kind === "rate" || (spec.kind === "quantile" && spec.gated);
  let status: ItemStatus =
    firstComparison.status === "not_compared" ? "not_compared"
    : firstComparison.status === "pass" ? "pass"
    : confirmed === null || confirmed.status === "not_compared" ? "fail"
    : confirmed.status === "fail" ? "fail" : "noisy";
  let limit: WaitMetricItem["limit"] = null;
  if (limitName !== undefined) {
    const value = limits[limitName] ?? null;
    const within = (figure: number | null): boolean | null => (value === null || figure === null ? null : figure <= value);
    limit = { name: limitName, value, baseline_within: within(firstComparison.baseline), candidate_within: within(firstComparison.candidate) };
    if (limit.candidate_within === false) status = "fail";
  }
  return { id, metric: spec.metric, unit: spec.kind === "rate" ? "rate" : "ms", gated: gated || limit?.value != null, first: firstComparison, confirm: confirmed, limit, status };
}

function itemHint(spec: ScenarioSpec, metrics: readonly WaitMetricItem[]): string | null {
  const failing = metrics.filter((m) => m.gated && (m.status === "fail" || m.status === "noisy"));
  const worst = failing.sort((a, b) => (b.first.ratio ?? 0) - (a.first.ratio ?? 0))[0];
  if (worst === undefined) return null;
  if (worst.limit?.candidate_within === false) return `${worst.metric} ${Math.round(worst.first.candidate ?? 0)} ms over the limit ${worst.limit.value} ms (${worst.limit.name})`;
  if (worst.unit === "rate") return `${worst.metric} ${((worst.first.baseline ?? 0) * 100).toFixed(1)}% → ${((worst.first.candidate ?? 0) * 100).toFixed(1)}%`;
  const ci = worst.first.ci === null ? "" : `, 95% CI ${worst.first.ci.lo.toFixed(2)}–${worst.first.ci.hi.toFixed(2)}×`;
  return `${spec.kind === "single" ? "median" : worst.metric} ${Math.round(worst.first.baseline ?? 0)} → ${Math.round(worst.first.candidate ?? 0)} ms${ci}`;
}

function notesOf(runs: readonly WaitRun[], side: WaitSide): Record<string, (number | string | boolean | null)[]> {
  const out: Record<string, (number | string | boolean | null)[]> = {};
  for (const run of runs.filter((r) => r.side === side)) for (const [key, value] of Object.entries(run.notes)) (out[key] ??= []).push(value);
  return out;
}

export function waitItem(spec: ScenarioSpec, runs: readonly WaitRun[], config: Pick<WaitsConfig, "limits" | "reportOnly" | "reproduce">): WaitItem {
  const id = scenarioId(spec);
  const own = runs.filter((r) => r.wait === spec.wait && r.scenario === spec.scenario);
  const first = own.filter((r) => r.phase === "runs");
  const confirm = own.filter((r) => r.phase === "confirm");
  const limitOf = new Map(Object.entries(spec.limits).map(([name, metric]) => [metric, name]));
  const metrics = spec.metrics.map((m) => metricItem(`${id}/${m.metric}`, m, first, confirm, config.limits, limitOf.get(m.metric)));
  const failed = own.filter((r) => !r.ok).map((r) => ({ side: r.side, rep: r.rep, reason: r.reason ?? "failed" }));
  const sideRuns = (side: WaitSide): number => first.filter((r) => r.side === side && r.ok).length;
  const counted = metrics.filter((m) => m.gated);
  let status = worstStatus(counted.map((m) => m.status));
  let reason: string | null = null;
  // A run the baseline completes and the candidate does not fails the scenario, as completion does for the packs.
  const baselineFailed = failed.filter((f) => f.side === "baseline").length;
  const candidateFailed = failed.filter((f) => f.side === "candidate").length;
  if (candidateFailed > baselineFailed) {
    status = "fail";
    reason = `the candidate failed ${candidateFailed} run(s): ${failed.find((f) => f.side === "candidate")!.reason}`;
  } else if (counted.every((m) => m.status === "not_compared")) {
    reason = own.length === 0 ? "not run" : `no completed runs to compare (${failed[0]?.reason ?? "none recorded"})`;
  }
  const sibling = own.reduce((most, r) => Math.max(most, r.siblingBusy), 0);
  const dropped = own.map((r) => r.cacheDropped).filter((d): d is boolean => d !== null);
  return {
    id, wait: spec.wait, scenario: spec.scenario, title: spec.title, entry: spec.entry, store: spec.store, kind: spec.kind, shape: spec.shape ?? null, step: spec.step ?? null,
    gated: !config.reportOnly.includes(spec.wait) && !config.reportOnly.includes(id),
    status, reason, metrics,
    runs: { baseline: sideRuns("baseline"), candidate: sideRuns("candidate"), confirm: confirm.length, failed },
    notes: { baseline: notesOf(own, "baseline"), candidate: notesOf(own, "candidate") },
    load: { baseline: own.filter((r) => r.side === "baseline").map((r) => r.load), candidate: own.filter((r) => r.side === "candidate").map((r) => r.load) },
    sibling_busy_max: sibling,
    cache_dropped: dropped.length === 0 ? null : dropped.every(Boolean),
    hint: reason ?? itemHint(spec, metrics),
    reproduce: config.reproduce(spec),
  };
}

/** The scenarios a confirmation should run again: those whose first runs fail a counted metric. */
export function failingScenarios(runs: readonly WaitRun[], config: Pick<WaitsConfig, "limits">): ScenarioSpec[] {
  const first = runs.filter((r) => r.phase === "runs");
  return SCENARIOS.filter((spec) => waitItem(spec, first, { ...config, reportOnly: [], reproduce: () => "" }).metrics.some((m) => m.gated && m.status === "fail"));
}

export function buildWaitsSection(runs: readonly WaitRun[], config: WaitsConfig): WaitsSection {
  const items = SCENARIOS.filter((spec) => config.requested.includes(scenarioId(spec))).map((spec) => waitItem(spec, runs, config));
  const measured = items.filter((i) => i.runs.baseline + i.runs.candidate > 0);
  const notMeasured = [
    ...items.filter((i) => i.runs.baseline + i.runs.candidate === 0).map((i) => ({ id: i.id, reason: config.skipped[i.id] ?? (i.runs.failed.length > 0 ? `every run failed on both sides: ${i.runs.failed[0]!.reason}` : "not run") })),
    ...Object.entries(config.skipped).filter(([id]) => !items.some((i) => i.id === id)).map(([id, reason]) => ({ id, reason })),
  ];
  const counted = measured.filter((i) => i.gated);
  const failing = counted.filter((i) => i.status === "fail").length;
  const status = failing > 0 ? "FAIL" : counted.some((i) => i.status === "not_compared") || measured.length < items.length ? "INCOMPLETE" : "PASS";
  return {
    status,
    counts: { scenarios: measured.length, failing, noisy: counted.filter((i) => i.status === "noisy").length },
    limits: config.limits,
    inputs: config.inputs,
    method: config.method,
    items: measured,
    not_measured: notMeasured,
  };
}
