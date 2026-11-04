/**
 * Merge Assist significance: the confirmatory analysis of `--arms` reports from benchmark/merge-assist-context.ts.
 *
 *   node --import tsx benchmark/merge-assist-stats.ts --analyze report.json [report.json ...] --arms A,B [--metric m] [--look N] [--seed N]
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
 * says whether to stop. Everything here is a pure function of the reports and a seed; nothing calls a model.
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
 * The secondary hypotheses: per-size P1–P3 (P3 still against its margin), then time superiority, uncached tokens and
 * churn over every task. Never used for stopping.
 */
const SECONDARY: readonly Hypothesis[] = [
  ...SIZES.flatMap(size => PRIMARY.slice(0, 3).map(hypothesis => ({ ...hypothesis, id: `${hypothesis.id}-${size}`, sizes: [size] }))),
  { id: "S-time", metric: "time_to_completion", sizes: null },
  { id: "S-uncached", metric: "uncached_tokens", sizes: null },
  { id: "S-churn", metric: "churn", sizes: null },
];

function formatEffect(metric: Metric, value: number): string {
  switch (METRIC_SPECS[metric].kind) {
    case "binary": return `${value >= 0 ? "+" : ""}${value.toFixed(3)}`;
    case "log": return `×${value.toFixed(3)}`;
    case "plain": return `${value >= 0 ? "+" : ""}${value.toFixed(1)}`;
  }
}

function describeEffect(metric: Metric, arms: readonly [string, string]): string {
  switch (METRIC_SPECS[metric].kind) {
    case "binary": return `mean within-task difference in proportions, ${arms[0]} − ${arms[1]}`;
    case "log": return `ratio of geometric means of value + 1, ${arms[0]}/${arms[1]}`;
    case "plain": return `mean within-task difference in lines, ${arms[0]} − ${arms[1]}`;
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
  readonly result: TestResult | null;
  readonly unpaired: readonly string[];
}

function analyse(hypotheses: readonly Hypothesis[], runs: readonly ArmRun[], arms: readonly [string, string], seed: number): Analysis[] {
  return hypotheses.map(hypothesis => {
    const scoped = runs.filter(run => hypothesis.sizes === null || (run.size !== null && hypothesis.sizes.includes(run.size)));
    const { strata, unpaired } = strataFor(hypothesis.metric, scoped, arms);
    return { hypothesis, result: strata.length === 0 ? null : testHypothesis(hypothesis.metric, strata, seed, hypothesis.margin), unpaired };
  });
}

function printAnalyses(title: string, analyses: readonly Analysis[], adjusted: ReadonlyArray<number | null>, arms: readonly [string, string]): void {
  console.log(`\n${title}`);
  console.log(table(
    ["hypothesis", "metric", "sizes", "n runs", "n tasks", "effect", "95% CI", "raw p", "Holm p", `decision (≤ ${POCOCK_ALPHA})`],
    analyses.map(({ hypothesis, result, unpaired }, index) => {
      const { metric } = hypothesis;
      const scope = hypothesis.sizes?.join("+") ?? "all";
      const better = hypothesis.margin === undefined
        ? `${arms[0]} ${METRIC_SPECS[metric].better}`
        : `non-inferiority: ${arms[0]}/${arms[1]} < margin ${formatEffect(metric, hypothesis.margin)}`;
      if (result === null) return [hypothesis.id, `${METRIC_SPECS[metric].label} (${better})`, scope, "0", "0", "-", "-", "-", "-", "no paired task"];
      const holmP = adjusted[index] ?? null;
      return [
        hypothesis.id, `${METRIC_SPECS[metric].label} (${better})`, scope, String(result.runs), `${result.tasks}${unpaired.length === 0 ? "" : ` (+${unpaired.length} unpaired)`}`,
        formatEffect(metric, result.effect), `[${formatEffect(metric, result.ci[0])}, ${formatEffect(metric, result.ci[1])}]`,
        formatP(result.p), holmP === null ? "-" : formatP(holmP), holmP === null ? (result.p <= POCOCK_ALPHA ? "(nominal) reject" : "(nominal) keep") : holmP <= POCOCK_ALPHA ? "REJECT H0" : "keep H0",
      ];
    }),
  ));
  for (const metric of [...new Set(analyses.map(({ hypothesis }) => hypothesis.metric))]) console.log(`  ${METRIC_SPECS[metric].label}: ${describeEffect(metric, arms)}`);
}

function analyzeReports(paths: readonly string[], arms: readonly [string, string], metric: Metric | null, look: number, seed: number): void {
  const reports = paths.map(path => ({ path, report: JSON.parse(readFileSync(path, "utf8")) as ArmsReport }));
  for (const { path, report } of reports) if (!Array.isArray(report.arms) || !Array.isArray(report.runs)) throw new Error(`${path} is not an --arms report of benchmark/merge-assist-context.ts`);
  const problems = poolingProblems(reports, arms);
  if (problems.length > 0) throw new Error(`Refusing to pool:\n- ${problems.join("\n- ")}`);
  const runs = reports.flatMap(({ report }) => report.runs.filter(run => arms.includes(run.arm)));
  const first = reports[0]!.report;
  console.log(`Merge Assist significance: ${arms[0]} (treatment) vs ${arms[1]} (control), look ${look} of at most ${MAX_LOOKS}; ${reports.length} report(s) pooled: ${reports.map(({ path, report }) => `${path} (${report.startedAt}, ${report.codexVersion})`).join("; ")}`);
  console.log(`Index ${first.index.path} built ${first.index.builtAt}; ${PROTOCOL_OPTIONS.map(option => `${option} ${JSON.stringify(first.options[option] ?? null)}`).join(", ")}; ${PERMUTATIONS} permutations within task, ${RESAMPLES} cluster-bootstrap resamples, seed ${seed}; one-sided`);
  const errors = runs.filter(failed);
  console.log(`\nExcluded: none. Runs with a Codex or harness error, counted as failures for their arm: ${arms.map(arm => `${arm} ${errors.filter(run => run.arm === arm).length}/${runs.filter(run => run.arm === arm).length}`).join(", ")}`);
  for (const run of errors) console.log(`  ${run.arm} ${run.taskId} #${run.trial}: ${(run.error ?? [run.phase1, ...run.rounds].find(phase => phase.error !== null)?.error ?? "").replace(/\s+/gu, " ").slice(0, 160)}`);
  const family = metric === null ? PRIMARY : [{ id: metric, metric, sizes: null }];
  const primary = analyse(family, runs, arms, seed);
  const adjusted = holm(primary.map(({ result }) => result?.p ?? 1));
  printAnalyses(metric === null ? `Primary family (Holm across ${family.length})` : `Metric ${metric}, every task`, primary, adjusted, arms);
  if (metric === null) printAnalyses("Secondary (not used for stopping; raw p against the per-look α)", analyse(SECONDARY, runs, arms, seed), SECONDARY.map(() => null), arms);
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

/**
 * `--self-test`: across 200 synthetic datasets each, the false-positive rate under no effect is at most 0.05 at
 * α 0.05, and under a large effect (+0.4 first-pass clean; time × 0.6) power is at least 0.9, for a binary and a
 * continuous metric; for time's non-inferiority test at margin ×{@link TIME_MARGIN}, the false-positive rate at a true
 * ratio of exactly the margin is at most 0.05, and the power at a true ratio of 0.9 is reported. All through the same
 * test and permutation count the analysis uses.
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
  console.log(`Self-test: ${datasets} synthetic datasets per case (18 tasks × 2 runs per arm), ${PERMUTATIONS} permutations each, α ${alpha} one-sided`);
  console.log(table(["case", "check", "rejected", "required", "", "time"], rows));
  const holmCheck = holm([0.01, 0.04, 0.03, 0.005]).map(p => p.toFixed(3)).join(",");
  const holmOk = holmCheck === "0.030,0.060,0.060,0.020";
  console.log(`Holm on [0.01, 0.04, 0.03, 0.005] = [${holmCheck}] ${holmOk ? "ok" : "FAIL (expected 0.030,0.060,0.060,0.020)"}`);
  console.log(ok && holmOk ? "self-test passed" : "self-test FAILED");
  return ok && holmOk;
}

// ---------------------------------------------------------------------------------------------------------- CLI

const USAGE = `Usage: node --import tsx benchmark/merge-assist-stats.ts --analyze report.json [report.json ...] --arms A,B [--metric ${METRICS.join("|")}] [--look N] [--seed N] | --analyze-live live.json [live.json ...] [--look N] [--planned-looks N] [--seed N] | --self-test`;

function main(): void {
  const args = process.argv.slice(2);
  if (args.includes("--self-test")) {
    process.exitCode = selfTest() ? 0 : 1;
    return;
  }
  const paths: string[] = [];
  const livePaths: string[] = [];
  let arms: [string, string] | null = null;
  let metric: Metric | null = null;
  let look: number | null = null;
  let plannedLooks = 1;
  let seed = 1;
  for (let index = 0; index < args.length; index++) {
    const flag = args[index]!;
    const value = (): string => {
      const next = args[++index];
      if (next === undefined || next.startsWith("--")) throw new Error(`${flag} takes a value\n${USAGE}`);
      return next;
    };
    switch (flag) {
      case "--analyze":
        while (args[index + 1] !== undefined && !args[index + 1]!.startsWith("--")) paths.push(args[++index]!);
        break;
      case "--analyze-live":
        while (args[index + 1] !== undefined && !args[index + 1]!.startsWith("--")) livePaths.push(args[++index]!);
        break;
      case "--planned-looks": {
        const number = Number(value());
        if (!Number.isInteger(number) || number < 1 || number > MAX_LOOKS) throw new Error(`--planned-looks takes 1 to ${MAX_LOOKS}\n${USAGE}`);
        plannedLooks = number;
        break;
      }
      case "--arms": {
        const names = value().split(",").map(name => name.trim()).filter(name => name !== "");
        if (names.length !== 2 || names[0] === names[1]) throw new Error(`--arms takes two different arms, treatment first: A,B\n${USAGE}`);
        arms = [names[0]!, names[1]!];
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
  if (paths.length === 0 || arms === null) throw new Error(USAGE);
  if (look === null && paths.length > MAX_LOOKS) throw new Error(`${paths.length} reports is more than ${MAX_LOOKS} looks; pass --look\n${USAGE}`);
  analyzeReports(paths, arms, metric, look ?? paths.length, seed);
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
