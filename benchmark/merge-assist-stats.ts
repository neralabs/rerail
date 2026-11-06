/**
 * Merge Assist significance: the confirmatory analysis of `--arms` reports from benchmark/merge-assist-context.ts.
 *
 *   node --import tsx benchmark/merge-assist-stats.ts --analyze report.json [report.json ...] --arms A,B [--metric m] [--look N] [--seed N]
 *   node --import tsx benchmark/merge-assist-stats.ts --analyze report.json [report.json ...] --family large --arms A,B,C[,D] [--diagnostic]
 *   node --import tsx benchmark/merge-assist-stats.ts --analyze-live live.json [live.json ...] [--look N] [--planned-looks N] [--seed N]
 *   node --import tsx benchmark/merge-assist-stats.ts --self-test
 *
 * The unit of analysis is a run, stratified by task: every comparison is within a task, then summed over tasks, so a
 * hard task weighs the same in both arms. The batches of several reports are pooled, one look per report unless
 * `--look` says otherwise; reports with a different index or arm definitions are refused. `A` is the treatment and
 * `B` the control: each hypothesis is one-sided, in the direction that favours `A`.
 *
 * - Binary metrics (`first_pass_clean`, `needs_fix_round`): a stratified permutation test on the summed within-task
 *   difference in proportions; the effect is the mean within-task difference, with a cluster-bootstrap 95% CI.
 * - Continuous metrics (`time_to_completion`, `total_tokens`, `uncached_tokens`): the same test on the within-task
 *   difference of mean log(value + 1); the effect is the ratio of geometric means (A/B, of value + 1). `churn` uses
 *   the plain difference of means, and its effect is that difference in lines.
 * - Non-inferiority against a margin (a ratio m, logged metrics only): H0 is ratio A/B ≥ m. A's log values are lowered
 *   by log(m) within each task, then the same one-sided test for "lower"; the reported ratio and CI are not shifted.
 * - A run with a Codex or harness error counts as a failure for its arm: not first-pass clean, needing a fix round,
 *   and at least as costly as the worst run of its task.
 *
 * Without `--metric`, the primary family of the pre-registration (benchmark/README.md): P1 first-pass clean higher,
 * P2 total tokens lower, P3 time to completion non-inferior at ×1.10, P4 needing a fix round lower on large tasks;
 * Holm-corrected, then the secondary hypotheses, never used for stopping. With `--metric`, that one metric over every
 * task, superiority. Each look is
 * judged against the Pocock boundary for at most three looks, nominal one-sided α = 0.0221 per look, and the last line
 * says whether to stop. The primary family refuses a diagnostic ceiling (`oracle-area`, which selects with the task's
 * labels) as the treatment unless `--diagnostic` is passed. Everything here is a pure function of the reports and a
 * seed; nothing calls a model.
 *
 * `--family large` is the large-task confirmation over `--arms A,B,C` (A the treatment, B posthoc, C the second
 * control): L1–L4 ({@link LARGE_PRIMARY}) with Holm and the same looks, then its secondaries; a 4th arm D with
 * `--diagnostic` adds D-vs-A headroom lines that decide nothing.
 *
 * `--analyze-live` reads `--live` reports (the two-agent coordination check): L1, B's violation of the decision lower
 * with `focus-pool` than with `none`, by the same stratified permutation test with strata = B task, the effect the
 * mean within-task difference in proportions with a cluster-bootstrap 95% CI. One-sided α 0.05 when the check is
 * looked at once (`--planned-looks 1`, the default), else the Pocock boundary per look, at most three looks.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { isDeepStrictEqual } from "node:util";
import { fileURLToPath } from "node:url";

/** The nominal one-sided α per look of a Pocock boundary for at most {@link MAX_LOOKS} looks at overall α 0.05. */
export const POCOCK_ALPHA = 0.0221;
export const MAX_LOOKS = 3;
export const PERMUTATIONS = 20_000;
export const RESAMPLES = 10_000;

export const METRICS = ["first_pass_clean", "needs_fix_round", "time_to_completion", "total_tokens", "uncached_tokens", "churn"] as const;
export type Metric = (typeof METRICS)[number];
const SIZES = ["small", "medium", "large"] as const;
type Size = (typeof SIZES)[number];

interface MetricSpec {
  readonly kind: "binary" | "log" | "plain";
  /** The direction that favours the treatment arm A. */
  readonly better: "higher" | "lower";
  readonly label: string;
}

export const METRIC_SPECS: Readonly<Record<Metric, MetricSpec>> = {
  first_pass_clean: { kind: "binary", better: "higher", label: "first-pass clean" },
  needs_fix_round: { kind: "binary", better: "lower", label: "needs a fix round" },
  time_to_completion: { kind: "log", better: "lower", label: "time to completion" },
  total_tokens: { kind: "log", better: "lower", label: "total tokens" },
  uncached_tokens: { kind: "log", better: "lower", label: "uncached tokens" },
  churn: { kind: "plain", better: "lower", label: "churn" },
};

// ---------------------------------------------------------------------------------------------------------- pure statistics

/** A small seeded generator (mulberry32), so every analysis of the same reports gives the same p-values and CIs. */
export function seeded(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4_294_967_296;
  };
}

/** One task's analysed values: the treatment arm's runs (`a`) and the control's (`b`), each at least one. */
export interface Stratum {
  readonly task: string;
  readonly a: readonly number[];
  readonly b: readonly number[];
}

const sum = (values: ArrayLike<number>): number => {
  let total = 0;
  for (let index = 0; index < values.length; index++) total += values[index]!;
  return total;
};
const mean = (values: readonly number[]): number => sum(values) / values.length;

/** The test statistic: the within-task difference of means (A − B), summed over tasks. */
export function stratifiedStatistic(strata: readonly Stratum[]): number {
  return strata.reduce((total, stratum) => total + mean(stratum.a) - mean(stratum.b), 0);
}

/**
 * The one-sided p-value of a stratified permutation test: the arm labels shuffled within each task, the group sizes
 * kept, `permutations` times. `direction` is the alternative: A's values `higher` or `lower` than B's. The observed
 * labelling counts as one permutation, so p is never 0.
 */
export function permutationTest(strata: readonly Stratum[], direction: "higher" | "lower", permutations: number, random: () => number): number {
  const observed = stratifiedStatistic(strata);
  const pools = strata.map(stratum => Float64Array.from([...stratum.a, ...stratum.b]));
  const sizes = strata.map(stratum => stratum.a.length);
  const totals = pools.map(pool => sum(pool));
  // Rounding in the sums must not turn a tie with the observed labelling into a miss.
  const tolerance = 1e-9 * Math.max(1, Math.abs(observed));
  let extreme = 0;
  for (let permutation = 0; permutation < permutations; permutation++) {
    let statistic = 0;
    for (let index = 0; index < pools.length; index++) {
      const pool = pools[index]!;
      const take = sizes[index]!;
      // A partial Fisher–Yates shuffle: the first `take` slots become a uniform random A group.
      let picked = 0;
      for (let slot = 0; slot < take; slot++) {
        const swap = slot + Math.floor(random() * (pool.length - slot));
        const value = pool[swap]!;
        pool[swap] = pool[slot]!;
        pool[slot] = value;
        picked += value;
      }
      statistic += picked / take - (totals[index]! - picked) / (pool.length - take);
    }
    if (direction === "higher" ? statistic >= observed - tolerance : statistic <= observed + tolerance) extreme++;
  }
  return (extreme + 1) / (permutations + 1);
}

/**
 * A cluster-bootstrap percentile 95% CI of `estimate`: tasks resampled with replacement, then each drawn task's runs
 * resampled with replacement within each arm, `resamples` times.
 */
export function clusterBootstrap(strata: readonly Stratum[], estimate: (strata: readonly Stratum[]) => number, resamples: number, random: () => number): [number, number] {
  const draw = (values: readonly number[]) => values.map(() => values[Math.floor(random() * values.length)]!);
  const estimates = Float64Array.from({ length: resamples }, () => estimate(strata.map(() => {
    const stratum = strata[Math.floor(random() * strata.length)]!;
    return { task: stratum.task, a: draw(stratum.a), b: draw(stratum.b) };
  }))).sort();
  return [quantile(estimates, 0.025), quantile(estimates, 0.975)];
}

/** The `q` quantile of sorted `values`, interpolated linearly. */
function quantile(values: Float64Array, q: number): number {
  const position = (values.length - 1) * q;
  const low = Math.floor(position);
  const high = Math.ceil(position);
  return values[low]! + (values[high]! - values[low]!) * (position - low);
}

/** Holm's step-down adjustment of a family of p-values, in the family's order. */
export function holm(pValues: readonly number[]): number[] {
  const order = pValues.map((p, index) => ({ p, index })).sort((left, right) => left.p - right.p);
  const adjusted = new Array<number>(pValues.length);
  let running = 0;
  order.forEach(({ p, index }, rank) => {
    running = Math.max(running, Math.min(1, (pValues.length - rank) * p));
    adjusted[index] = running;
  });
  return adjusted;
}

/** The values the test compares: 0/1 for a binary metric, log(value + 1) for a logged one, the value for churn. */
export function analysedValue(metric: Metric, value: number): number {
  return METRIC_SPECS[metric].kind === "log" ? Math.log1p(value) : value;
}

/**
 * The reported effect over `strata` of analysed values: the mean within-task difference (A − B) of proportions or
 * of churn, and for a logged metric the ratio of geometric means (A/B, of value + 1).
 */
export function effect(metric: Metric, strata: readonly Stratum[]): number {
  const difference = stratifiedStatistic(strata) / strata.length;
  return METRIC_SPECS[metric].kind === "log" ? Math.exp(difference) : difference;
}

export interface TestResult {
  readonly runs: number;
  readonly tasks: number;
  readonly effect: number;
  readonly ci: readonly [number, number];
  readonly p: number;
}

/**
 * The analysed values the permutation test compares. With a non-inferiority `margin` (a ratio, logged metrics only),
 * A's log values are lowered by log(margin) within each task, so "A lower" tests H0: ratio A/B ≥ margin.
 */
export function testedStrata(metric: Metric, raw: readonly Stratum[], margin = 1): Stratum[] {
  if (margin !== 1 && METRIC_SPECS[metric].kind !== "log") throw new Error(`A non-inferiority margin needs a logged metric, not ${metric}`);
  const shift = Math.log(margin);
  return raw.map(stratum => ({
    task: stratum.task,
    a: stratum.a.map(value => analysedValue(metric, value) - shift),
    b: stratum.b.map(value => analysedValue(metric, value)),
  }));
}

/**
 * One hypothesis over `strata` of raw values (already per task and arm): the test (against `margin` when one is
 * given), the effect and its CI (never shifted).
 */
export function testHypothesis(metric: Metric, raw: readonly Stratum[], seed: number, margin = 1, permutations = PERMUTATIONS, resamples = RESAMPLES): TestResult {
  const strata = testedStrata(metric, raw);
  const random = seeded(seed);
  const p = permutationTest(margin === 1 ? strata : testedStrata(metric, raw, margin), METRIC_SPECS[metric].better, permutations, random);
  const ci = clusterBootstrap(strata, picked => effect(metric, picked), resamples, random);
  return { runs: strata.reduce((total, stratum) => total + stratum.a.length + stratum.b.length, 0), tasks: strata.length, effect: effect(metric, strata), ci, p };
}

export type Verdict = "STOP: success" | "CONTINUE" | "STOP: futility/max looks";

/** The look's verdict: success when every primary rejects at its Holm-adjusted p; at the last look, stop regardless. */
export function verdict(adjusted: readonly number[], look: number): Verdict {
  if (adjusted.length > 0 && adjusted.every(p => p <= POCOCK_ALPHA)) return "STOP: success";
  return look >= MAX_LOOKS ? "STOP: futility/max looks" : "CONTINUE";
}

// ---------------------------------------------------------------------------------------------------------- reports

interface Usage {
  readonly input: number;
  readonly cached: number;
  readonly output: number;
}

interface PhaseCall {
  readonly error: string | null;
}

/** The fields of an `--arms` run the analysis reads. */
interface ArmRun {
  readonly arm: string;
  readonly taskId: string;
  readonly size: Size | null;
  readonly trial: number;
  readonly phase1: PhaseCall;
  readonly rounds: readonly PhaseCall[];
  readonly firstPassClean: boolean;
  readonly churn: number;
  readonly wallMs: number;
  readonly tokens: Usage;
  readonly error: string | null;
}

interface ArmsReport {
  readonly startedAt: string;
  readonly codexVersion: string;
  readonly options: Readonly<Record<string, unknown>>;
  readonly index: { readonly path: string; readonly builtAt: string };
  readonly arms: readonly string[];
  readonly armDefinitions?: Readonly<Record<string, unknown>>;
  readonly maxFixRounds?: number;
  readonly runs: readonly ArmRun[];
}

/** The options of a run that define what an arm is, besides its parameters: they must match to pool. */
const PROTOCOL_OPTIONS = ["distractors", "seed", "timeoutMin", "model", "config"] as const;

/** Why `reports` cannot be pooled for `arms`: a different index, arm definitions or protocol; empty when they can. */
export function poolingProblems(reports: ReadonlyArray<{ path: string; report: ArmsReport }>, arms: readonly string[]): string[] {
  const problems: string[] = [];
  const [first, ...rest] = reports;
  if (first === undefined) return ["no report"];
  for (const { path, report } of reports) {
    const missing = arms.filter(arm => !report.arms.includes(arm));
    if (missing.length > 0) problems.push(`${path} has no runs of arm ${missing.join(", ")} (its arms: ${report.arms.join(", ")})`);
  }
  const starts = new Set(reports.map(({ report }) => report.startedAt));
  if (starts.size < reports.length) problems.push("the same batch appears twice (two reports share a start time)");
  for (const { path, report } of rest) {
    if (!isDeepStrictEqual(report.index, first.report.index)) problems.push(`${path} ran on index ${report.index.path} built ${report.index.builtAt}, ${first.path} on ${first.report.index.path} built ${first.report.index.builtAt}`);
    for (const option of PROTOCOL_OPTIONS) {
      if (!isDeepStrictEqual(report.options[option], first.report.options[option])) problems.push(`${path} ran with ${option} ${JSON.stringify(report.options[option])}, ${first.path} with ${JSON.stringify(first.report.options[option])}`);
    }
    for (const arm of arms) {
      const [left, right] = [first.report.armDefinitions?.[arm], report.armDefinitions?.[arm]];
      if (left === undefined || right === undefined) problems.push(`arm ${arm}: ${left === undefined ? first.path : path} does not record its definition, so the reports cannot be shown to share it`);
      else if (!isDeepStrictEqual(left, right)) problems.push(`arm ${arm} is defined differently in ${path} and ${first.path}`);
    }
    if (report.maxFixRounds !== first.report.maxFixRounds) problems.push(`${path} allows ${report.maxFixRounds ?? "unrecorded"} fix rounds, ${first.path} ${first.report.maxFixRounds ?? "unrecorded"}`);
  }
  return problems;
}

/** A run that hit a Codex or harness error, in a phase or around it. */
const failed = (run: ArmRun): boolean => run.error !== null || [run.phase1, ...run.rounds].some(phase => phase.error !== null);

/** A run's raw value of `metric`, as measured (errors not yet counted as failures). */
function measured(metric: Metric, run: ArmRun): number {
  switch (metric) {
    case "first_pass_clean": return run.firstPassClean ? 1 : 0;
    case "needs_fix_round": return run.rounds.length > 0 ? 1 : 0;
    case "time_to_completion": return run.wallMs / 1000;
    case "total_tokens": return run.tokens.input + run.tokens.output;
    case "uncached_tokens": return run.tokens.input - run.tokens.cached;
    case "churn": return run.churn;
  }
}

/**
 * Per task, A's and B's values of `metric` over `runs`, errors counted as failures: 0 for first-pass clean, 1 for
 * needing a fix round, and for a cost the worse of its own value and the worst error-free run of its task. Tasks
 * without a run in both arms are left out.
 */
export function strataFor(metric: Metric, runs: readonly ArmRun[], arms: readonly [string, string]): { strata: Stratum[]; unpaired: string[] } {
  const tasks = [...new Set(runs.map(run => run.taskId))];
  const strata: Stratum[] = [];
  const unpaired: string[] = [];
  for (const task of tasks) {
    const own = runs.filter(run => run.taskId === task && arms.includes(run.arm));
    const clean = own.filter(run => !failed(run)).map(run => measured(metric, run));
    const worst = clean.length === 0 ? 0 : Math.max(...clean);
    const value = (run: ArmRun): number => {
      if (!failed(run)) return measured(metric, run);
      if (metric === "first_pass_clean") return 0;
      if (metric === "needs_fix_round") return 1;
      return Math.max(measured(metric, run), worst);
    };
    const [a, b] = arms.map(arm => own.filter(run => run.arm === arm).map(value)) as [number[], number[]];
    if (a.length === 0 || b.length === 0) unpaired.push(task);
    else strata.push({ task, a, b });
  }
  return { strata, unpaired };
}

interface Hypothesis {
  readonly id: string;
  readonly metric: Metric;
  /** The sizes it covers; null for every task. */
  readonly sizes: readonly Size[] | null;
  /** Non-inferiority: H0 is ratio A/B ≥ this margin, rejected when A is shown below it. Absent for superiority. */
  readonly margin?: number;
  /** The compared arms as positions in `--arms`, treatment first; absent for the first two. */
  readonly pair?: readonly [number, number];
}

/** The non-inferiority margin of time to completion: the treatment may take at most 10% longer. */
export const TIME_MARGIN = 1.1;

/** The pre-registered primary family (benchmark/README.md): the hypotheses the stopping rule reads. */
export const PRIMARY: readonly Hypothesis[] = [
  { id: "P1", metric: "first_pass_clean", sizes: null },
  { id: "P2", metric: "total_tokens", sizes: null },
  { id: "P3", metric: "time_to_completion", sizes: null, margin: TIME_MARGIN },
  { id: "P4", metric: "needs_fix_round", sizes: ["large"] },
];

/**
 * Arms that select with the task's labels (merge-assist-context.ts `oracle-area`): diagnostic ceilings, which the
 * primary family refuses as the treatment unless `--diagnostic` says the analysis is one.
 */
export const DIAGNOSTIC_ARMS: readonly string[] = ["oracle-area"];

/**
 * The secondary hypotheses: per-size P1–P3 (P3 still against its margin), then time superiority, uncached tokens and
 * churn over every task. Never used for stopping.
 */
const SECONDARY: readonly Hypothesis[] = [
  ...SIZES.flatMap(size => PRIMARY.slice(0, 3).map(hypothesis => ({ ...hypothesis, id: `${hypothesis.id}-${size}`, sizes: [size] }))),
  { id: "S-time", metric: "time_to_completion", sizes: null },
  { id: "S-uncached", metric: "uncached_tokens", sizes: null },
  { id: "S-churn", metric: "churn", sizes: null },
];

export const FAMILIES = ["default", "large"] as const;
export type Family = (typeof FAMILIES)[number];

/**
 * The pre-registered large-task confirmation (`--family large`, `--arms A,B,C`): A the treatment, B posthoc, C the
 * best earlier protocol. Holm across exactly these four.
 */
export const LARGE_PRIMARY: readonly Hypothesis[] = [
  { id: "L1", metric: "first_pass_clean", sizes: ["large"], pair: [0, 1] },
  { id: "L2", metric: "total_tokens", sizes: ["large"], pair: [0, 1] },
  { id: "L3", metric: "time_to_completion", sizes: ["large"], margin: TIME_MARGIN, pair: [0, 1] },
  { id: "L4", metric: "first_pass_clean", sizes: ["large"], pair: [0, 2] },
];

/** The large family's secondary hypotheses, raw p only. */
const LARGE_SECONDARY: readonly Hypothesis[] = [
  { id: "LS-tokens-C", metric: "total_tokens", sizes: ["large"], pair: [0, 2] },
  { id: "LS-time-C", metric: "time_to_completion", sizes: ["large"], margin: TIME_MARGIN, pair: [0, 2] },
  { id: "LS-fix-B", metric: "needs_fix_round", sizes: ["large"], pair: [0, 1] },
  { id: "LS-fix-C", metric: "needs_fix_round", sizes: ["large"], pair: [0, 2] },
  { id: "LS-uncached-B", metric: "uncached_tokens", sizes: ["large"], pair: [0, 1] },
  { id: "LS-churn-B", metric: "churn", sizes: ["large"], pair: [0, 1] },
];

/** `--family large --diagnostic` with a 4th arm D (`oracle-area`): how far D is ahead of A. Raw p; decides nothing. */
const LARGE_HEADROOM: readonly Hypothesis[] = [
  { id: "H-clean", metric: "first_pass_clean", sizes: ["large"], pair: [3, 0] },
  { id: "H-tokens", metric: "total_tokens", sizes: ["large"], pair: [3, 0] },
  { id: "H-time", metric: "time_to_completion", sizes: ["large"], pair: [3, 0] },
];

function formatEffect(kind: MetricSpec["kind"], value: number): string {
  switch (kind) {
    case "binary": return `${value >= 0 ? "+" : ""}${value.toFixed(3)}`;
    case "log": return `×${value.toFixed(3)}`;
    case "plain": return `${value >= 0 ? "+" : ""}${value.toFixed(1)}`;
  }
}

function describeEffect(metric: Metric, [a, b]: readonly [string, string]): string {
  switch (METRIC_SPECS[metric].kind) {
    case "binary": return `mean within-task difference in proportions, ${a} − ${b}`;
    case "log": return `ratio of geometric means of value + 1, ${a}/${b}`;
    case "plain": return `mean within-task difference in lines, ${a} − ${b}`;
  }
}

function table(headers: readonly string[], rows: ReadonlyArray<readonly string[]>): string {
  const widths = headers.map((header, column) => Math.max(header.length, ...rows.map(row => (row[column] ?? "").length)));
  const line = (cells: readonly string[]) => cells.map((cell, column) => cell.padEnd(widths[column]!)).join("  ").trimEnd();
  return [line(headers), line(widths.map(width => "-".repeat(width))), ...rows.map(line)].join("\n");
}

const formatP = (p: number): string => (p < 0.0001 ? p.toExponential(1) : p.toFixed(4));

interface Analysis {
  readonly hypothesis: Hypothesis;
  /** The compared arms by name, treatment first. */
  readonly pair: readonly [string, string];
  readonly result: TestResult | null;
  readonly unpaired: readonly string[];
}

/** A hypothesis's arms by name and its strata of raw values: its sizes' runs of those two arms, per task. */
function hypothesisStrata(hypothesis: Hypothesis, runs: readonly ArmRun[], arms: readonly string[]): { pair: [string, string]; strata: Stratum[]; unpaired: string[] } {
  const [a, b] = hypothesis.pair ?? [0, 1];
  const pair: [string, string] = [arms[a]!, arms[b]!];
  const scoped = runs.filter(run => hypothesis.sizes === null || (run.size !== null && hypothesis.sizes.includes(run.size)));
  return { pair, ...strataFor(hypothesis.metric, scoped, pair) };
}

function analyse(hypotheses: readonly Hypothesis[], runs: readonly ArmRun[], arms: readonly string[], seed: number): Analysis[] {
  return hypotheses.map(hypothesis => {
    const { pair, strata, unpaired } = hypothesisStrata(hypothesis, runs, arms);
    return { hypothesis, pair, result: strata.length === 0 ? null : testHypothesis(hypothesis.metric, strata, seed, hypothesis.margin), unpaired };
  });
}

/** `adjusted` holds each row's Holm p (null: raw p only); `diagnostic` rows show no decision at all. */
function printAnalyses(title: string, analyses: readonly Analysis[], adjusted: ReadonlyArray<number | null>, diagnostic = false): void {
  console.log(`\n${title}`);
  console.log(table(
    ["hypothesis", "metric", "sizes", "n runs", "n tasks", "effect", "95% CI", "raw p", "Holm p", `decision (≤ ${POCOCK_ALPHA})`],
    analyses.map(({ hypothesis, pair, result, unpaired }, index) => {
      const { metric } = hypothesis;
      const scope = hypothesis.sizes?.join("+") ?? "all";
      const better = hypothesis.margin === undefined
        ? `${pair[0]} ${METRIC_SPECS[metric].better} than ${pair[1]}`
        : `non-inferiority: ${pair[0]}/${pair[1]} < margin ${formatEffect(METRIC_SPECS[metric].kind, hypothesis.margin)}`;
      if (result === null) return [hypothesis.id, `${METRIC_SPECS[metric].label} (${better})`, scope, "0", "0", "-", "-", "-", "-", "no paired task"];
      const holmP = adjusted[index] ?? null;
      return [
        hypothesis.id, `${METRIC_SPECS[metric].label} (${better})`, scope, String(result.runs), `${result.tasks}${unpaired.length === 0 ? "" : ` (+${unpaired.length} unpaired)`}`,
        formatEffect(METRIC_SPECS[metric].kind, result.effect), `[${formatEffect(METRIC_SPECS[metric].kind, result.ci[0])}, ${formatEffect(METRIC_SPECS[metric].kind, result.ci[1])}]`,
        formatP(result.p), holmP === null ? "-" : formatP(holmP),
        diagnostic ? "decides nothing" : holmP === null ? (result.p <= POCOCK_ALPHA ? "(nominal) reject" : "(nominal) keep") : holmP <= POCOCK_ALPHA ? "REJECT H0" : "keep H0",
      ];
    }),
  ));
  for (const line of new Set(analyses.map(({ hypothesis, pair }) => `${METRIC_SPECS[hypothesis.metric].label}: ${describeEffect(hypothesis.metric, pair)}`))) console.log(`  ${line}`);
}

/**
 * `--analyze`: the family's primary hypotheses with Holm and the look's verdict, then its secondaries. `arms` is A,B
 * for the default family and A,B,C for `large`; a 4th arm D is analysed only under `headroom` and when every report
 * holds it.
 */
function analyzeReports(paths: readonly string[], arms: readonly string[], family: Family, metric: Metric | null, look: number, seed: number, headroom: boolean): void {
  const reports = paths.map(path => ({ path, report: JSON.parse(readFileSync(path, "utf8")) as ArmsReport }));
  for (const { path, report } of reports) if (!Array.isArray(report.arms) || !Array.isArray(report.runs)) throw new Error(`${path} is not an --arms report of benchmark/merge-assist-context.ts`);
  const diagnosticArm = family === "large" && arms.length === 4 ? arms[3]! : null;
  const withHeadroom = diagnosticArm !== null && headroom && reports.every(({ report }) => report.arms.includes(diagnosticArm));
  const used = diagnosticArm !== null && !withHeadroom ? arms.slice(0, 3) : arms;
  const problems = poolingProblems(reports, used);
  if (problems.length > 0) throw new Error(`Refusing to pool:\n- ${problems.join("\n- ")}`);
  const runs = reports.flatMap(({ report }) => report.runs.filter(run => used.includes(run.arm)));
  const first = reports[0]!.report;
  const compared = family === "large" ? `${arms[0]} (treatment) vs ${arms[1]} (control) and ${arms[2]} (second control)` : `${arms[0]} (treatment) vs ${arms[1]} (control)`;
  console.log(`Merge Assist significance, family ${family}: ${compared}, look ${look} of at most ${MAX_LOOKS}; ${reports.length} report(s) pooled: ${reports.map(({ path, report }) => `${path} (${report.startedAt}, ${report.codexVersion})`).join("; ")}`);
  console.log(`Index ${first.index.path} built ${first.index.builtAt}; ${PROTOCOL_OPTIONS.map(option => `${option} ${JSON.stringify(first.options[option] ?? null)}`).join(", ")}; ${PERMUTATIONS} permutations within task, ${RESAMPLES} cluster-bootstrap resamples, seed ${seed}; one-sided`);
  if (diagnosticArm !== null && !withHeadroom) console.log(headroom ? `${diagnosticArm} is not in every report: no headroom lines.` : `${diagnosticArm} ignored: headroom lines need --diagnostic.`);
  const errors = runs.filter(failed);
  console.log(`\nExcluded: none. Runs with a Codex or harness error, counted as failures for their arm: ${used.map(arm => `${arm} ${errors.filter(run => run.arm === arm).length}/${runs.filter(run => run.arm === arm).length}`).join(", ")}`);
  for (const run of errors) console.log(`  ${run.arm} ${run.taskId} #${run.trial}: ${(run.error ?? [run.phase1, ...run.rounds].find(phase => phase.error !== null)?.error ?? "").replace(/\s+/gu, " ").slice(0, 160)}`);
  const primaryFamily = family === "large" ? LARGE_PRIMARY : metric === null ? PRIMARY : [{ id: metric, metric, sizes: null }];
  const primary = analyse(primaryFamily, runs, used, seed);
  const adjusted = holm(primary.map(({ result }) => result?.p ?? 1));
  printAnalyses(metric === null ? `Primary family ${family} (Holm across ${primaryFamily.length})` : `Metric ${metric}, every task`, primary, adjusted);
  const secondary = family === "large" ? LARGE_SECONDARY : metric === null ? SECONDARY : [];
  if (secondary.length > 0) printAnalyses("Secondary (not used for stopping; raw p against the per-look α)", analyse(secondary, runs, used, seed), secondary.map(() => null));
  if (withHeadroom) printAnalyses(`Headroom: ${diagnosticArm} vs ${arms[0]} (diagnostic, raw p, decides nothing)`, analyse(LARGE_HEADROOM, runs, used, seed), LARGE_HEADROOM.map(() => null), true);
  console.log(`\n${verdict(adjusted, look)}`);
}

// ---------------------------------------------------------------------------------------------------------- live

/** The live check's treatment and control arms (merge-assist-context.ts `--live`). */
export const LIVE_ARMS = ["focus-pool", "none"] as const;
/** The live check's one-sided α when it is looked at once. */
export const SINGLE_LOOK_ALPHA = 0.05;

/** The α per look of a live check planned for `plannedLooks` looks: 0.05 for one, the Pocock boundary for more. */
export function liveAlpha(plannedLooks: number): number {
  if (!Number.isInteger(plannedLooks) || plannedLooks < 1 || plannedLooks > MAX_LOOKS) throw new Error(`A live check plans 1 to ${MAX_LOOKS} looks`);
  return plannedLooks === 1 ? SINGLE_LOOK_ALPHA : POCOCK_ALPHA;
}

/** The fields of a live B run the analysis reads. */
export interface LiveRun {
  readonly arm: string;
  readonly taskId: string;
  /** D's oracle found a violation in B's diff. */
  readonly violated: boolean;
  readonly error: string | null;
}

export interface LiveResult extends TestResult {
  readonly unpaired: readonly string[];
  /** Violated runs (errors counted) over runs, per arm: `a` focus-pool, `b` none. */
  readonly rates: { readonly a: { readonly hits: number; readonly total: number }; readonly b: { readonly hits: number; readonly total: number } };
  readonly alpha: number;
  readonly reject: boolean;
}

/**
 * L1 over `runs`: per B task, focus-pool's and none's violation of D (1 violated, 0 not; a run with an error counts as
 * violated), tested one-sided for focus-pool lower; null when no B task ran in both arms.
 */
export function liveTest(runs: readonly LiveRun[], seed: number, alpha: number, permutations = PERMUTATIONS, resamples = RESAMPLES): LiveResult | null {
  const value = (run: LiveRun): number => (run.violated || run.error !== null ? 1 : 0);
  const strata: Stratum[] = [];
  const unpaired: string[] = [];
  for (const task of [...new Set(runs.map(run => run.taskId))]) {
    const [a, b] = LIVE_ARMS.map(arm => runs.filter(run => run.taskId === task && run.arm === arm).map(value)) as [number[], number[]];
    if (a.length === 0 || b.length === 0) unpaired.push(task);
    else strata.push({ task, a, b });
  }
  if (strata.length === 0) return null;
  const random = seeded(seed);
  const p = permutationTest(strata, "lower", permutations, random);
  const estimate = (picked: readonly Stratum[]) => stratifiedStatistic(picked) / picked.length;
  const ci = clusterBootstrap(strata, estimate, resamples, random);
  const paired = strata.flatMap(stratum => [...stratum.a.map(hit => ({ arm: "a", hit })), ...stratum.b.map(hit => ({ arm: "b", hit }))]);
  const counted = (arm: string) => ({ hits: paired.filter(entry => entry.arm === arm && entry.hit === 1).length, total: paired.filter(entry => entry.arm === arm).length });
  return { runs: paired.length, tasks: strata.length, effect: estimate(strata), ci, p, unpaired, rates: { a: counted("a"), b: counted("b") }, alpha, reject: p <= alpha };
}

/** The fields of a `--live` report the analysis reads. */
interface LiveReport {
  readonly format: string;
  readonly startedAt: string;
  readonly codexVersion: string;
  readonly options: Readonly<Record<string, unknown>>;
  readonly index: { readonly path: string; readonly builtAt: string };
  readonly variants?: Readonly<Record<string, unknown>>;
  readonly trials: ReadonlyArray<{ readonly decisionId: string; readonly capture: { readonly outcome: string }; readonly b: readonly LiveRun[] }>;
}

const LIVE_REPORT_FORMAT = "rerail.merge-assist-live/v1";

function analyzeLiveReports(paths: readonly string[], look: number, plannedLooks: number, seed: number): void {
  const reports = paths.map(path => ({ path, report: JSON.parse(readFileSync(path, "utf8")) as LiveReport }));
  const problems: string[] = [];
  for (const { path, report } of reports) if (report.format !== LIVE_REPORT_FORMAT || !Array.isArray(report.trials)) problems.push(`${path} is not a --live report of benchmark/merge-assist-context.ts`);
  if (new Set(reports.map(({ report }) => report.startedAt)).size < reports.length) problems.push("the same batch appears twice (two reports share a start time)");
  const [first, ...rest] = reports;
  for (const { path, report } of rest) {
    if (!isDeepStrictEqual(report.index, first!.report.index)) problems.push(`${path} ran on index ${report.index.path} built ${report.index.builtAt}, ${first!.path} on ${first!.report.index.path} built ${first!.report.index.builtAt}`);
    if (!isDeepStrictEqual(report.variants, first!.report.variants)) problems.push(`${path} defines the arms differently from ${first!.path}`);
    for (const option of PROTOCOL_OPTIONS) {
      if (!isDeepStrictEqual(report.options[option], first!.report.options[option])) problems.push(`${path} ran with ${option} ${JSON.stringify(report.options[option])}, ${first!.path} with ${JSON.stringify(first!.report.options[option])}`);
    }
  }
  if (problems.length > 0) throw new Error(`Refusing to pool:\n- ${problems.join("\n- ")}`);
  if (look > plannedLooks) throw new Error(`Look ${look} is past the ${plannedLooks} planned; plan the looks before the first one (--planned-looks)`);
  const trials = reports.flatMap(({ report }) => report.trials);
  const runs = trials.flatMap(trial => trial.b.map(run => ({ ...run, captured: trial.capture.outcome === "captured" })));
  const alpha = liveAlpha(plannedLooks);
  console.log(`Merge Assist live check: L1, B's violation of the decision, focus-pool (treatment) vs none (control), look ${look} of ${plannedLooks} planned; ${reports.length} report(s) pooled: ${reports.map(({ path, report }) => `${path} (${report.startedAt}, ${report.codexVersion})`).join("; ")}`);
  console.log(`Index ${first!.report.index.path} built ${first!.report.index.builtAt}; ${PROTOCOL_OPTIONS.map(option => `${option} ${JSON.stringify(first!.report.options[option] ?? null)}`).join(", ")}; ${PERMUTATIONS} permutations within B task, ${RESAMPLES} cluster-bootstrap resamples, seed ${seed}; one-sided α ${alpha}${plannedLooks === 1 ? " (one look)" : " (Pocock)"}`);
  const captured = trials.filter(trial => trial.capture.outcome === "captured").length;
  console.log(`\nCapture: ${captured}/${trials.length} trials captured the decision. Excluded: none; B runs with an error count as violations: ${LIVE_ARMS.map(arm => `${arm} ${runs.filter(run => run.arm === arm && run.error !== null).length}/${runs.filter(run => run.arm === arm).length}`).join(", ")}`);
  console.log(table(
    ["arm", "violated: all", "captured", "not captured"],
    LIVE_ARMS.map(arm => {
      const cell = (list: ReadonlyArray<LiveRun>) => `${list.filter(run => run.violated || run.error !== null).length}/${list.length}`;
      const own = runs.filter(run => run.arm === arm);
      return [arm, cell(own), cell(own.filter(run => run.captured)), cell(own.filter(run => !run.captured))];
    }),
  ));
  const result = liveTest(runs, seed, alpha);
  if (result === null) {
    console.log("\nNo B task ran in both arms.");
    return;
  }
  console.log(`\nL1 over ${result.runs} runs, ${result.tasks} B tasks${result.unpaired.length === 0 ? "" : ` (+${result.unpaired.length} unpaired: ${result.unpaired.join(", ")})`}: effect ${result.effect >= 0 ? "+" : ""}${result.effect.toFixed(3)} (mean within-task difference in proportions, focus-pool − none), 95% CI [${result.ci[0].toFixed(3)}, ${result.ci[1].toFixed(3)}], p ${formatP(result.p)}: ${result.reject ? "REJECT H0" : "keep H0"}`);
  console.log(`\n${result.reject ? "STOP: success" : look >= plannedLooks ? "STOP: futility/max looks" : "CONTINUE"}`);
}

// ---------------------------------------------------------------------------------------------------------- replay

/** The fields of a merge-assist-replay.ts run (`ReplayRun`) the analysis reads. */
export interface ReplayRunFields {
  readonly pair: string;
  readonly arm: string;
  readonly trial: number;
  readonly error: string | null;
  readonly leaked: boolean;
  readonly wallMs: number;
  readonly usage: Usage | null;
  readonly diff: string;
  readonly merge: { readonly head: { readonly files: readonly string[] } } | null;
  readonly touches: { readonly firstHalf: readonly string[] };
  readonly grade: { readonly verdict: { readonly conflicts_with_first: "yes" | "no" | "unclear"; readonly done: "yes" | "partial" | "no" } | null; readonly error: string | null } | null;
  readonly git: { readonly checks: number; readonly detections: ReadonlyArray<{ readonly resolvedAtS: number | null }>; readonly openAtEnd: number } | null;
}

/** The fields of a merge-assist-replay.ts `--json` report the analysis reads. */
interface ReplayReport {
  readonly format: string;
  readonly options: Readonly<Record<string, unknown>>;
  readonly pairs: ReadonlyArray<{ readonly id: string; readonly pair: unknown; readonly spec: unknown }>;
  readonly runs: readonly ReplayRunFields[];
}

const REPLAY_REPORT_FORMAT = "rerail.merge-assist-replay/v1";
/** The replay options that define what an arm and its grading are: they must match to pool. */
const REPLAY_PROTOCOL_OPTIONS = ["dataset", "provider", "model", "judgeModel", "config", "timeoutMin", "judgeTimeoutMin", "alignModule"] as const;

export const REPLAY_METRICS = ["conflict", "done", "total_tokens", "wall_time", "merge_conflict_files", "first_half_files"] as const;
export type ReplayMetric = (typeof REPLAY_METRICS)[number];

export const REPLAY_SPECS: Readonly<Record<ReplayMetric, MetricSpec>> = {
  conflict: { kind: "binary", better: "lower", label: "conflicts with first (grader yes or unclear)" },
  done: { kind: "binary", better: "higher", label: "done (yes 1, partial 0.5, no 0)" },
  total_tokens: { kind: "log", better: "lower", label: "total tokens" },
  wall_time: { kind: "log", better: "lower", label: "wall time" },
  merge_conflict_files: { kind: "plain", better: "lower", label: "merge-tree conflict files vs first's head" },
  first_half_files: { kind: "plain", better: "lower", label: "first_half files touched" },
};

/** A replay run that failed: a run error, an empty diff, or no grader verdict. It counts as conflicting and not done. */
export const replayFailed = (run: ReplayRunFields): boolean => run.error !== null || run.diff.trim() === "" || run.grade?.verdict == null;

function replayMeasured(metric: ReplayMetric, run: ReplayRunFields): number {
  switch (metric) {
    case "conflict": return run.grade?.verdict?.conflicts_with_first === "no" ? 0 : 1;
    case "done": return { yes: 1, partial: 0.5, no: 0 }[run.grade?.verdict?.done ?? "no"];
    case "total_tokens": return run.usage === null ? 0 : run.usage.input + run.usage.output;
    case "wall_time": return run.wallMs / 1000;
    case "merge_conflict_files": return run.merge?.head.files.length ?? 0;
    case "first_half_files": return run.touches.firstHalf.length;
  }
}

/**
 * Per replay pair, the treatment's and the control's values of `metric` over `runs` (leaked runs already left out).
 * A failed run conflicts (1) and is not done (0); for a cost it is at least as costly as its pair's worst sound run.
 */
export function replayStrata(metric: ReplayMetric, runs: readonly ReplayRunFields[], treatment: string, control: string): { strata: Stratum[]; unpaired: string[] } {
  const strata: Stratum[] = [];
  const unpaired: string[] = [];
  for (const pair of [...new Set(runs.map(run => run.pair))]) {
    const own = runs.filter(run => run.pair === pair && (run.arm === treatment || run.arm === control));
    const sound = own.filter(run => !replayFailed(run)).map(run => replayMeasured(metric, run));
    const worst = sound.length === 0 ? 0 : Math.max(...sound);
    const value = (run: ReplayRunFields): number => {
      if (!replayFailed(run)) return replayMeasured(metric, run);
      if (metric === "conflict") return 1;
      if (metric === "done") return 0;
      return Math.max(replayMeasured(metric, run), worst);
    };
    const a = own.filter(run => run.arm === treatment).map(value);
    const b = own.filter(run => run.arm === control).map(value);
    if (a.length === 0 || b.length === 0) unpaired.push(pair);
    else strata.push({ task: pair, a, b });
  }
  return { strata, unpaired };
}

/** One replay hypothesis: the one-sided stratified permutation test and the effect with its cluster-bootstrap CI. */
export function replayTest(metric: ReplayMetric, raw: readonly Stratum[], seed: number, permutations = PERMUTATIONS, resamples = RESAMPLES): TestResult {
  const { kind, better } = REPLAY_SPECS[metric];
  const analysed = (value: number) => (kind === "log" ? Math.log1p(value) : value);
  const strata = raw.map(stratum => ({ task: stratum.task, a: stratum.a.map(analysed), b: stratum.b.map(analysed) }));
  const estimate = (picked: readonly Stratum[]) => {
    const difference = stratifiedStatistic(picked) / picked.length;
    return kind === "log" ? Math.exp(difference) : difference;
  };
  const random = seeded(seed);
  const p = permutationTest(strata, better, permutations, random);
  const ci = clusterBootstrap(strata, estimate, resamples, random);
  return { runs: strata.reduce((total, stratum) => total + stratum.a.length + stratum.b.length, 0), tasks: strata.length, effect: estimate(strata), ci, p };
}

/** Why `reports` cannot be pooled: not replay reports, the same batch twice, or a different protocol or pair spec. */
function replayPoolingProblems(reports: ReadonlyArray<{ path: string; report: ReplayReport }>): string[] {
  const problems: string[] = [];
  for (const { path, report } of reports) if (report.format !== REPLAY_REPORT_FORMAT || !Array.isArray(report.runs)) problems.push(`${path} is not a --json report of benchmark/merge-assist-replay.ts`);
  if (problems.length > 0) return problems;
  if (new Set(reports.map(({ path }) => resolve(path))).size < reports.length) problems.push("the same report appears twice");
  const seen = new Map<string, string>();
  for (const { path, report } of reports) {
    for (const run of report.runs) {
      const key = `${run.pair}\0${run.arm}\0${run.trial}\0${run.wallMs}`;
      const other = seen.get(key);
      if (other !== undefined && other !== path) problems.push(`${path} and ${other} hold the same run (${run.pair} ${run.arm} #${run.trial}): the same batch twice`);
      seen.set(key, path);
    }
  }
  const [first, ...rest] = reports;
  for (const { path, report } of rest) {
    for (const option of REPLAY_PROTOCOL_OPTIONS) {
      if (!isDeepStrictEqual(report.options[option], first!.report.options[option])) problems.push(`${path} ran with ${option} ${JSON.stringify(report.options[option])}, ${first!.path} with ${JSON.stringify(first!.report.options[option])}`);
    }
  }
  const specs = new Map<string, { path: string; pair: unknown; spec: unknown }>();
  for (const { path, report } of reports) {
    for (const entry of report.pairs) {
      const known = specs.get(entry.id);
      if (known === undefined) specs.set(entry.id, { path, pair: entry.pair, spec: entry.spec });
      else if (!isDeepStrictEqual(known.pair, entry.pair) || !isDeepStrictEqual(known.spec, entry.spec)) problems.push(`pair ${entry.id} is defined differently in ${path} and ${known.path}`);
    }
  }
  return problems;
}

interface ReplayAnalysis {
  readonly id: string;
  readonly metric: ReplayMetric;
  readonly treatment: string;
  readonly result: TestResult | null;
  readonly unpaired: readonly string[];
}

function analyseReplay(id: string, metric: ReplayMetric, runs: readonly ReplayRunFields[], treatment: string, control: string, seed: number): ReplayAnalysis {
  const { strata, unpaired } = replayStrata(metric, runs, treatment, control);
  return { id, metric, treatment, result: strata.length === 0 ? null : replayTest(metric, strata, seed), unpaired };
}

function printReplay(title: string, analyses: readonly ReplayAnalysis[], control: string, adjusted: ReadonlyArray<number | null>): void {
  console.log(`\n${title}`);
  console.log(table(
    ["hypothesis", "metric", "n runs", "n pairs", "effect", "95% CI", "raw p", "Holm p", `decision (≤ ${POCOCK_ALPHA})`],
    analyses.map(({ id, metric, treatment, result, unpaired }, index) => {
      const { kind, better, label } = REPLAY_SPECS[metric];
      const name = `${label} (${treatment} ${better} than ${control}${metric === "done" ? "; not worse: read the CI" : ""})`;
      if (result === null) return [id, name, "0", "0", "-", "-", "-", "-", "no paired pair"];
      const holmP = adjusted[index] ?? null;
      return [
        id, name, String(result.runs), `${result.tasks}${unpaired.length === 0 ? "" : ` (+${unpaired.length} unpaired)`}`,
        formatEffect(kind, result.effect), `[${formatEffect(kind, result.ci[0])}, ${formatEffect(kind, result.ci[1])}]`,
        formatP(result.p), holmP === null ? "-" : formatP(holmP),
        holmP === null ? (result.p <= POCOCK_ALPHA ? "(nominal) reject" : "(nominal) keep") : holmP <= POCOCK_ALPHA ? "REJECT H0" : "keep H0",
      ];
    }),
  ));
}

/**
 * `--analyze-replay`: per treatment arm against `control`, R1 (conflicts with first lower) with Holm across the
 * treatments and the look's verdict; then the secondaries, raw p, and what the `git-conflict` arms identified.
 */
function analyzeReplayReports(paths: readonly string[], arms: readonly string[], control: string, look: number, seed: number): void {
  const reports = paths.map(path => ({ path, report: JSON.parse(readFileSync(path, "utf8")) as ReplayReport }));
  const problems = replayPoolingProblems(reports);
  const all = reports.flatMap(({ report }) => report.runs);
  const treatments = arms.filter(arm => arm !== control);
  for (const arm of [control, ...treatments]) if (!all.some(run => run.arm === arm)) problems.push(`no report holds a run of arm ${arm}`);
  if (treatments.length === 0) problems.push(`--arms names no treatment besides the control ${control}`);
  if (problems.length > 0) throw new Error(`Refusing to pool:\n- ${problems.join("\n- ")}`);
  const used = [control, ...treatments];
  const listed = all.filter(run => used.includes(run.arm));
  const runs = listed.filter(run => !run.leaked);
  const options = reports[0]!.report.options;
  console.log(`Merge Assist replay: ${treatments.join(", ")} (treatment${treatments.length === 1 ? "" : "s"}) vs ${control} (control), strata = replay pair, look ${look} of at most ${MAX_LOOKS}; ${reports.length} report(s) pooled: ${paths.join("; ")}`);
  console.log(`${REPLAY_PROTOCOL_OPTIONS.map(option => `${option} ${JSON.stringify(options[option] ?? null)}`).join(", ")}; ${PERMUTATIONS} permutations within pair, ${RESAMPLES} cluster-bootstrap resamples, seed ${seed}; one-sided`);
  console.log(`\nExcluded (leaked): ${used.map(arm => `${arm} ${listed.filter(run => run.arm === arm && run.leaked).length}/${listed.filter(run => run.arm === arm).length}`).join(", ")}`);
  const failures = runs.filter(replayFailed);
  console.log(`Failures (a run error, an empty diff or no verdict), counted as conflicting and not done: ${used.map(arm => `${arm} ${failures.filter(run => run.arm === arm).length}/${runs.filter(run => run.arm === arm).length}`).join(", ")}`);
  for (const run of failures) console.log(`  ${run.arm} ${run.pair} #${run.trial}: ${(run.error ?? (run.diff.trim() === "" ? "empty diff" : run.grade?.error ?? "no grader verdict")).replace(/\s+/gu, " ").slice(0, 160)}`);
  const primary = treatments.map((arm, index) => analyseReplay(`R1-${index + 1}`, "conflict", runs, arm, control, seed));
  const adjusted = holm(primary.map(({ result }) => result?.p ?? 1));
  printReplay(`Primary: R1 per treatment (Holm across ${treatments.length})`, primary, control, adjusted);
  const secondaryMetrics = REPLAY_METRICS.filter(metric => metric !== "conflict");
  const secondary = treatments.flatMap((arm, index) => secondaryMetrics.map(metric => analyseReplay(`S${index + 1}-${metric}`, metric, runs, arm, control, seed)));
  printReplay("Secondary (not used for stopping; raw p against the per-look α)", secondary, control, secondary.map(() => null));
  const gitArms = treatments.filter(arm => runs.some(run => run.arm === arm && run.git !== null));
  if (gitArms.length > 0) {
    console.log("\nWhat the git-conflict arms identified (descriptive; the control identifies nothing)");
    console.log(table(
      ["arm", "runs", "ran git conflicts", "detections", "resolved", "open at end"],
      gitArms.map(arm => {
        const own = runs.filter(run => run.arm === arm && run.git !== null);
        const total = (pick: (git: NonNullable<ReplayRunFields["git"]>) => number) => own.reduce((sum, run) => sum + pick(run.git!), 0);
        return [arm, String(own.length), `${own.filter(run => run.git!.checks > 0).length}/${own.length}`, String(total(git => git.detections.length)), String(total(git => git.detections.filter(detection => detection.resolvedAtS !== null).length)), String(total(git => git.openAtEnd))];
      }),
    ));
  }
  console.log(`\n${verdict(adjusted, look)}`);
}

// ---------------------------------------------------------------------------------------------------------- self-test

/** A standard normal draw (Box–Muller) from `random`. */
function normal(random: () => number): number {
  return Math.sqrt(-2 * Math.log(1 - random())) * Math.cos(2 * Math.PI * random());
}

/**
 * Synthetic data shaped like the confirm split's first look: 18 tasks, 2 runs per arm per task, tasks of very
 * different difficulty. `shift` is the treatment's effect: added to each task's clean rate (binary), or to each run's
 * log time (continuous).
 */
function synthetic(kind: "binary" | "log", shift: number, random: () => number): Stratum[] {
  return Array.from({ length: 18 }, (_, task) => {
    if (kind === "binary") {
      const base = 0.1 + 0.5 * random();
      const draw = (rate: number) => (random() < rate ? 1 : 0);
      return { task: `t${task}`, a: [draw(base + shift), draw(base + shift)], b: [draw(base), draw(base)] };
    }
    const centre = Math.log(300) + 0.8 * normal(random);
    const draw = (offset: number) => Math.exp(centre + offset + 0.4 * normal(random));
    return { task: `t${task}`, a: [draw(shift), draw(shift)], b: [draw(0), draw(0)] };
  });
}

/** The synthetic large family's shape: the large dev batch's 9 tasks with 3 runs per arm. */
const LARGE_TASKS = 9;
const LARGE_RUNS = 3;

/**
 * Synthetic `--arms` runs of three arms on large tasks under the large family's global null: first-pass clean and
 * tokens the same in every arm, A's time exactly ×{@link TIME_MARGIN} B's (L3's boundary), C the same as B. Tasks
 * differ widely in difficulty and cost.
 */
function syntheticLargeRuns(arms: readonly string[], random: () => number): ArmRun[] {
  return Array.from({ length: LARGE_TASKS }, (_, task) => {
    const cleanRate = 0.1 + 0.5 * random();
    const tokens = Math.log(400_000) + 0.6 * normal(random);
    const seconds = Math.log(600) + 0.6 * normal(random);
    return arms.flatMap(arm => Array.from({ length: LARGE_RUNS }, (_, trial): ArmRun => {
      const firstPassClean = random() < cleanRate;
      const timeShift = arm === arms[0] ? Math.log(TIME_MARGIN) : 0;
      return {
        arm, taskId: `t${task}`, size: "large", trial: trial + 1, phase1: { error: null }, rounds: firstPassClean ? [] : [{ error: null }], firstPassClean, churn: 0,
        wallMs: 1000 * Math.exp(seconds + timeShift + 0.4 * normal(random)),
        tokens: { input: Math.round(Math.exp(tokens + 0.4 * normal(random))), cached: 0, output: 0 },
        error: null,
      };
    }));
  }).flat();
}

/** The synthetic replay batch's shape: 14 replay pairs with 2 runs per arm. */
const REPLAY_PAIRS = 14;
const REPLAY_RUNS = 2;

/**
 * Synthetic replay runs under no effect: every arm shares each pair's conflict rate (0.2 to 0.8), and every run has
 * the same 10% chance of failing (no diff) and 5% of leaking.
 */
function syntheticReplayRuns(arms: readonly string[], random: () => number): ReplayRunFields[] {
  return Array.from({ length: REPLAY_PAIRS }, (_, pair) => {
    const conflictRate = 0.2 + 0.6 * random();
    return arms.flatMap(arm => Array.from({ length: REPLAY_RUNS }, (_, trial): ReplayRunFields => {
      const empty = random() < 0.1;
      return {
        pair: `p${pair}`, arm, trial: trial + 1, error: null, leaked: random() < 0.05, wallMs: 60_000, usage: null, diff: empty ? "" : "diff", merge: null, touches: { firstHalf: [] },
        grade: { verdict: { conflicts_with_first: random() < conflictRate ? "yes" : "no", done: "yes" }, error: null }, git: null,
      };
    }));
  }).flat();
}

/**
 * `--self-test`: across 200 synthetic datasets each, the false-positive rate under no effect is at most 0.05 at
 * α 0.05, and under a large effect (+0.4 first-pass clean; time × 0.6) power is at least 0.9, for a binary and a
 * continuous metric; for time's non-inferiority test at margin ×{@link TIME_MARGIN}, the false-positive rate at a true
 * ratio of exactly the margin is at most 0.05, and the power at a true ratio of 0.9 is reported. The large family
 * (A,B,C) under its global null rejects any hypothesis after Holm at most 5% of the time, and so does the replay R1
 * (one treatment, and two with Holm; failures and leaks included). All through the same test, arm pairing and
 * permutation count the analysis uses.
 */
function selfTest(): boolean {
  const datasets = 200;
  const alpha = 0.05;
  const atMost = { required: "≤ 0.05", pass: (rate: number) => rate <= 0.05 };
  const atLeast = { required: "≥ 0.9", pass: (rate: number) => rate >= 0.9 };
  const reported = { required: "reported", pass: () => true };
  const cases = [
    { name: "binary first_pass_clean, no effect", metric: "first_pass_clean" as const, shift: 0, margin: 1, check: "false-positive rate", ...atMost },
    { name: "binary first_pass_clean, +0.4", metric: "first_pass_clean" as const, shift: 0.4, margin: 1, check: "power", ...atLeast },
    { name: "continuous time_to_completion, no effect", metric: "time_to_completion" as const, shift: 0, margin: 1, check: "false-positive rate", ...atMost },
    { name: "continuous time_to_completion, ×0.6", metric: "time_to_completion" as const, shift: Math.log(0.6), margin: 1, check: "power", ...atLeast },
    { name: `time non-inferiority ×${TIME_MARGIN}, true ×${TIME_MARGIN}`, metric: "time_to_completion" as const, shift: Math.log(TIME_MARGIN), margin: TIME_MARGIN, check: "false-positive rate", ...atMost },
    { name: `time non-inferiority ×${TIME_MARGIN}, true ×0.9`, metric: "time_to_completion" as const, shift: Math.log(0.9), margin: TIME_MARGIN, check: "power", ...reported },
  ];
  const random = seeded(20_261_104);
  let ok = true;
  const rows = cases.map(({ name, metric, shift, margin, check, required, pass }) => {
    const started = performance.now();
    let rejections = 0;
    for (let dataset = 0; dataset < datasets; dataset++) {
      const raw = synthetic(METRIC_SPECS[metric].kind === "binary" ? "binary" : "log", shift, random);
      if (permutationTest(testedStrata(metric, raw, margin), METRIC_SPECS[metric].better, PERMUTATIONS, random) <= alpha) rejections++;
    }
    const rate = rejections / datasets;
    const passed = pass(rate);
    ok &&= passed;
    return [name, check, `${rejections}/${datasets} = ${rate.toFixed(3)}`, required, required === "reported" ? "" : passed ? "ok" : "FAIL", `${((performance.now() - started) / 1000).toFixed(1)}s`];
  });
  const started = performance.now();
  const arms = ["A", "B", "C"];
  const rawRejections = LARGE_PRIMARY.map(() => 0);
  let familyRejections = 0;
  for (let dataset = 0; dataset < datasets; dataset++) {
    const runs = syntheticLargeRuns(arms, random);
    const pValues = LARGE_PRIMARY.map(hypothesis => {
      const { strata } = hypothesisStrata(hypothesis, runs, arms);
      return permutationTest(testedStrata(hypothesis.metric, strata, hypothesis.margin), METRIC_SPECS[hypothesis.metric].better, PERMUTATIONS, random);
    });
    pValues.forEach((p, index) => { if (p <= alpha) rawRejections[index]!++; });
    if (holm(pValues).some(p => p <= alpha)) familyRejections++;
  }
  const familyRate = familyRejections / datasets;
  ok &&= familyRate <= 0.05;
  rows.push([
    `large family A,B,C, global null (${LARGE_TASKS} tasks × ${LARGE_RUNS} runs per arm)`, "family-wise false-positive rate, Holm", `${familyRejections}/${datasets} = ${familyRate.toFixed(3)}`, "≤ 0.05",
    familyRate <= 0.05 ? "ok" : "FAIL", `${((performance.now() - started) / 1000).toFixed(1)}s`,
  ]);
  const replayStarted = performance.now();
  const replayArms = ["none", "T1", "T2"];
  let singleRejections = 0;
  let replayFamilyRejections = 0;
  for (let dataset = 0; dataset < datasets; dataset++) {
    const runs = syntheticReplayRuns(replayArms, random).filter(run => !run.leaked);
    const pValues = replayArms.slice(1).map(arm => permutationTest(replayStrata("conflict", runs, arm, "none").strata, "lower", PERMUTATIONS, random));
    if (pValues[0]! <= alpha) singleRejections++;
    if (holm(pValues).some(p => p <= alpha)) replayFamilyRejections++;
  }
  const replayShape = `${REPLAY_PAIRS} pairs × ${REPLAY_RUNS} runs per arm`;
  for (const [name, rejections] of [[`replay R1, T1 vs none, no effect (${replayShape})`, singleRejections], [`replay R1, T1,T2 vs none, no effect (${replayShape})`, replayFamilyRejections]] as const) {
    const rate = rejections / datasets;
    ok &&= rate <= 0.05;
    rows.push([name, name.includes("T1,T2") ? "family-wise false-positive rate, Holm" : "false-positive rate", `${rejections}/${datasets} = ${rate.toFixed(3)}`, "≤ 0.05", rate <= 0.05 ? "ok" : "FAIL", `${((performance.now() - replayStarted) / 1000).toFixed(1)}s`]);
  }
  console.log(`Self-test: ${datasets} synthetic datasets per case (18 tasks × 2 runs per arm unless stated), ${PERMUTATIONS} permutations each, α ${alpha} one-sided`);
  console.log(table(["case", "check", "rejected", "required", "", "time"], rows));
  console.log(`Large family, raw rejections per hypothesis at α ${alpha}: ${LARGE_PRIMARY.map((hypothesis, index) => `${hypothesis.id} ${rawRejections[index]}/${datasets}`).join(", ")}`);
  const holmCheck = holm([0.01, 0.04, 0.03, 0.005]).map(p => p.toFixed(3)).join(",");
  const holmOk = holmCheck === "0.030,0.060,0.060,0.020";
  console.log(`Holm on [0.01, 0.04, 0.03, 0.005] = [${holmCheck}] ${holmOk ? "ok" : "FAIL (expected 0.030,0.060,0.060,0.020)"}`);
  console.log(ok && holmOk ? "self-test passed" : "self-test FAILED");
  return ok && holmOk;
}

// ---------------------------------------------------------------------------------------------------------- CLI

const USAGE = `Usage: node --import tsx benchmark/merge-assist-stats.ts --analyze report.json [report.json ...] --arms A,B [--metric ${METRICS.join("|")}] [--look N] [--seed N] [--diagnostic] | --analyze report.json [report.json ...] --family large --arms A,B,C[,D] [--diagnostic] [--look N] [--seed N] | --analyze-live live.json [live.json ...] [--look N] [--planned-looks N] [--seed N] | --analyze-replay replay.json [replay.json ...] --arms A[,B...] --control none [--look N] [--seed N] | --self-test`;

function main(): void {
  const args = process.argv.slice(2);
  if (args.includes("--self-test")) {
    process.exitCode = selfTest() ? 0 : 1;
    return;
  }
  const paths: string[] = [];
  const livePaths: string[] = [];
  const replayPaths: string[] = [];
  let control: string | null = null;
  let arms: string[] | null = null;
  let family: Family = "default";
  let metric: Metric | null = null;
  let look: number | null = null;
  let plannedLooks = 1;
  let seed = 1;
  let diagnostic = false;
  for (let index = 0; index < args.length; index++) {
    const flag = args[index]!;
    const value = (): string => {
      const next = args[++index];
      if (next === undefined || next.startsWith("--")) throw new Error(`${flag} takes a value\n${USAGE}`);
      return next;
    };
    switch (flag) {
      case "--diagnostic":
        diagnostic = true;
        break;
      case "--analyze":
        while (args[index + 1] !== undefined && !args[index + 1]!.startsWith("--")) paths.push(args[++index]!);
        break;
      case "--analyze-live":
        while (args[index + 1] !== undefined && !args[index + 1]!.startsWith("--")) livePaths.push(args[++index]!);
        break;
      case "--analyze-replay":
        while (args[index + 1] !== undefined && !args[index + 1]!.startsWith("--")) replayPaths.push(args[++index]!);
        break;
      case "--control":
        control = value();
        break;
      case "--planned-looks": {
        const number = Number(value());
        if (!Number.isInteger(number) || number < 1 || number > MAX_LOOKS) throw new Error(`--planned-looks takes 1 to ${MAX_LOOKS}\n${USAGE}`);
        plannedLooks = number;
        break;
      }
      case "--arms": {
        const names = value().split(",").map(name => name.trim()).filter(name => name !== "");
        if (names.length === 0 || names.length > 4 || new Set(names).size < names.length) throw new Error(`--arms takes up to four different arms, treatment first: A,B (default family), A,B,C[,D] (large) or the treatments of a replay\n${USAGE}`);
        arms = names;
        break;
      }
      case "--family": {
        const name = value();
        if (!FAMILIES.some(known => known === name)) throw new Error(`--family takes one of ${FAMILIES.join(", ")}\n${USAGE}`);
        family = name as Family;
        break;
      }
      case "--metric": {
        const name = value();
        if (!METRICS.some(known => known === name)) throw new Error(`--metric takes one of ${METRICS.join(", ")}\n${USAGE}`);
        metric = name as Metric;
        break;
      }
      case "--look": {
        const number = Number(value());
        if (!Number.isInteger(number) || number < 1 || number > MAX_LOOKS) throw new Error(`--look takes 1 to ${MAX_LOOKS}\n${USAGE}`);
        look = number;
        break;
      }
      case "--seed": {
        const number = Number(value());
        if (!Number.isInteger(number)) throw new Error(`--seed takes an integer\n${USAGE}`);
        seed = number;
        break;
      }
      default:
        throw new Error(`Unknown argument ${flag}\n${USAGE}`);
    }
  }
  if (livePaths.length > 0) {
    if (paths.length > 0 || arms !== null || metric !== null) throw new Error(`--analyze-live takes no --analyze, --arms or --metric\n${USAGE}`);
    analyzeLiveReports(livePaths, look ?? livePaths.length, plannedLooks, seed);
    return;
  }
  if (replayPaths.length > 0) {
    if (paths.length > 0 || livePaths.length > 0 || metric !== null || family !== "default") throw new Error(`--analyze-replay takes no --analyze, --analyze-live, --metric or --family\n${USAGE}`);
    if (arms === null || control === null) throw new Error(`--analyze-replay takes --arms and --control\n${USAGE}`);
    if (look === null && replayPaths.length > MAX_LOOKS) throw new Error(`${replayPaths.length} reports is more than ${MAX_LOOKS} looks; pass --look\n${USAGE}`);
    analyzeReplayReports(replayPaths, arms, control, look ?? replayPaths.length, seed);
    return;
  }
  if (control !== null) throw new Error(`--control belongs to --analyze-replay\n${USAGE}`);
  if (paths.length === 0 || arms === null) throw new Error(USAGE);
  if (family === "default" && arms.length !== 2) throw new Error(`The default family takes two arms: --arms A,B\n${USAGE}`);
  if (family === "large" && arms.length < 3) throw new Error(`--family large takes --arms A,B,C (treatment, posthoc, second control), and D for --diagnostic headroom\n${USAGE}`);
  if (family === "large" && metric !== null) throw new Error(`--metric belongs to the default family\n${USAGE}`);
  if (look === null && paths.length > MAX_LOOKS) throw new Error(`${paths.length} reports is more than ${MAX_LOOKS} looks; pass --look\n${USAGE}`);
  if (metric === null && DIAGNOSTIC_ARMS.includes(arms[0]!)) {
    if (!diagnostic) throw new Error(`${arms[0]} selects with the task's labels: a diagnostic ceiling, never the treatment of the pre-registered primary family. Pass --diagnostic to analyse it as a diagnostic anyway\n${USAGE}`);
    console.log(`DIAGNOSTIC analysis: ${arms[0]} selects with the task's labels; this is not the pre-registered comparison and decides nothing.\n`);
  }
  analyzeReports(paths, arms, family, metric, look ?? paths.length, seed, diagnostic);
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
