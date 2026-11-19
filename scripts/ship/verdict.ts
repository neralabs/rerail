// The shippability bar's verdict logic (scripts/ship/shipbar.ts): pure functions over measured runs and findings, the
// same for every candidate. A candidate passes a pack's slices when every figure's median is at most 1.15 times main's
// (SLICES_TOLERANCE_PERCENT), keeps every finding main gets right and adds no false positive main does not have,
// completes every run main completes, and ships every rule main ships. Slice B (each rule alone) counts a rule over the
// bar only when it is still over after its confirmation runs and its time moved by at least RULE_FLOOR_MS; slice C (one
// file, all rules) is judged as a whole-pack cell is. Every failing time carries a locator: the phase that moved
// (locate). The user waits (waits-model.ts) keep their own bar, TOLERANCE_PERCENT.

/** The waits' tolerance, in percent: a candidate figure fails when it is more than this much above main's. */
export const TOLERANCE_PERCENT = 10;
/** The slices' tolerance (A, B and C of every pack), in percent. */
export const SLICES_TOLERANCE_PERCENT = 15;

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
  // Scaled to integers' arithmetic so the bar itself (1.10 or 1.15 times main) passes, not a float next to it.
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
  /** |ratio - bar| is at most the larger spread: the verdict could go either way between runs. */
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
 * or the reason it cannot run (its `.rerail` fails `check-rr`, …); a rule main ships that `candidate` lacks has none.
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

// ---------------------------------------------------------------- chunked runs

/** One process of a chunked run: the pack's rules split into fixed chunks, each chunk its own run-policies process. */
export interface ChunkRun {
  chunk: number;
  rep: number;
  mode: "cold" | "warm";
  completed: boolean;
  wallMs: number;
  cpuMs: number;
  rssKB: number;
}

/**
 * Per repetition and mode, the pack's figure from its chunks: wall and CPU summed, peak RSS the largest chunk's. A chunk
 * that failed any cold run is left out of every repetition (listed by the caller); a repetition counts only when every
 * other chunk completed it, so each sum covers the same rules.
 */
export function chunkSums(runs: readonly ChunkRun[], chunks: number): { mode: "cold" | "warm"; rep: number; wallMs: number; cpuMs: number; rssKB: number }[] {
  const failed = new Set(runs.filter((r) => r.mode === "cold" && !r.completed).map((r) => r.chunk));
  const kept = [...Array(chunks).keys()].filter((chunk) => !failed.has(chunk));
  const sums = [];
  for (const mode of ["cold", "warm"] as const) {
    for (const rep of [...new Set(runs.map((r) => r.rep))].sort((a, b) => a - b)) {
      const these = kept.map((chunk) => runs.find((r) => r.chunk === chunk && r.rep === rep && r.mode === mode && r.completed));
      if (kept.length === 0 || these.some((r) => r === undefined)) continue;
      const done = these as ChunkRun[];
      sums.push({ mode, rep, wallMs: done.reduce((s, r) => s + r.wallMs, 0), cpuMs: done.reduce((s, r) => s + r.cpuMs, 0), rssKB: Math.max(...done.map((r) => r.rssKB)) });
    }
  }
  return sums;
}

/**
 * How run-policies ran a pack's policies, from its result's `runs` (each with `together`: how many policies ran in its
 * group, 1 for a policy run alone): combined when any group holds several. An engine that reports no `together` leaves
 * it unknown.
 */
export function grouping(runs: readonly { together?: number }[]): { policies: number; combined: number; alone: number; largest: number } | null {
  if (runs.length === 0 || runs.some((r) => typeof r.together !== "number")) return null;
  const sizes = runs.map((r) => r.together!);
  return { policies: runs.length, combined: sizes.filter((n) => n > 1).length, alone: sizes.filter((n) => n === 1).length, largest: Math.max(...sizes) };
}

// ---------------------------------------------------------------- slice B: one rule × all files

/** A time over the bar counts only when it moved by at least this much (ms), after its confirmation runs. */
export const RULE_FLOOR_MS = 20;
/** Rules over this ratio (and moved by at least RULE_FLOOR_MS) are counted apart in the report. */
export const RULE_ALERT_RATIO = 1.5;

/**
 * pass: within the bar at the first runs; noisy: over at the first runs, within at the confirmation runs; under-floor:
 * still over, but moved by less than the floor; fail: still over and moved by at least the floor.
 */
export type TimeStatus = "pass" | "fail" | "noisy" | "under-floor" | "not-compared";

export interface TimeVerdict {
  name: string;
  /** The first runs' medians (ms); null where that side completed none. */
  main: number | null;
  candidate: number | null;
  ratio: number | null;
  /** The confirmation runs' medians, where both sides made some. */
  confirm: { main: number; candidate: number; ratio: number } | null;
  /** Over the bar at the first runs' medians: before confirmation and the floor (the report's unfiltered list). */
  overBefore: boolean;
  /** The figures the status rests on: the confirmation runs' where made, else the first runs'. */
  finalRatio: number | null;
  finalDeltaMs: number | null;
  status: TimeStatus;
}

/** One time (a rule's compile + evaluate, or a fixed-cost figure) against main's, with its confirmation and the floor. */
export function judgeTime(name: string, main: readonly number[], candidate: readonly number[], confirmMain: readonly number[] = [], confirmCandidate: readonly number[] = [], floorMs = RULE_FLOOR_MS, tolerancePercent = SLICES_TOLERANCE_PERCENT): TimeVerdict {
  const first = compareMetric(name, main, candidate, tolerancePercent);
  const base = { name, main: first.main, candidate: first.candidate, ratio: first.ratio };
  if (first.status === "not-compared") return { ...base, confirm: null, overBefore: false, finalRatio: null, finalDeltaMs: null, status: "not-compared" };
  const confirmed = confirmMain.length > 0 && confirmCandidate.length > 0 ? compareMetric(name, confirmMain, confirmCandidate, tolerancePercent) : null;
  const final = confirmed ?? first;
  const finalDeltaMs = final.candidate! - final.main!;
  const overBefore = first.status === "fail";
  let status: TimeStatus;
  if (!overBefore) status = "pass";
  else if (confirmed !== null && confirmed.status === "pass") status = "noisy";
  else status = finalDeltaMs >= floorMs ? "fail" : "under-floor";
  return { ...base, confirm: confirmed === null ? null : { main: confirmed.main!, candidate: confirmed.candidate!, ratio: confirmed.ratio! }, overBefore, finalRatio: final.ratio, finalDeltaMs, status };
}

/**
 * Whether a time needs confirmation runs: over the bar at its first runs, by at least the floor (one moved by less
 * cannot count whatever its confirmation says), and not confirmed yet.
 */
export function needsConfirmation(v: TimeVerdict, floorMs = RULE_FLOOR_MS): boolean {
  return v.overBefore && v.confirm === null && (v.finalDeltaMs ?? 0) >= floorMs;
}

/** Slice B's time of a rule on the `.rerail` side, where run-policies reports no phases: its net wall (netWalls). */
export const RR_RULE_TIME = "net-wall";

/**
 * A `.rerail` rule's net walls: each cold wall of the rule alone less the median cold wall of the empty rule set (the
 * fixed cost: start-up and reading the corpus), never below 0. None without an empty rule set's run to subtract.
 */
export function netWalls(walls: readonly number[], emptyWalls: readonly number[]): number[] {
  if (emptyWalls.length === 0) return [];
  const fixed = median(emptyWalls);
  return walls.map((wall) => Math.max(0, wall - fixed));
}

// ---------------------------------------------------------------- the `.rerail` side's gate

/** docs/rule-syntax/packs/<pack>/equivalence.json's format: the translation suite's declaration of a pack. */
export const EQUIVALENCE_FORMAT = "rerail.translation-equivalence/v1";

/**
 * Whether the `.rerail` side's verdict on a pack counts toward the exit status: only where the pack's equivalence.json
 * (`declaration`, its parsed JSON; undefined when the file is missing) is of EQUIVALENCE_FORMAT, names the pack, and
 * says `equivalent: true`. Otherwise it is reported only, with the reason.
 */
export function translationGate(pack: string, declaration: unknown): { counted: boolean; reason: string } {
  const file = `docs/rule-syntax/packs/${pack}/equivalence.json`;
  if (declaration === undefined) return { counted: false, reason: `reported only: no ${file}` };
  const d = declaration as { format?: unknown; pack?: unknown; equivalent?: unknown; different?: unknown };
  if (typeof d !== "object" || d === null || d.format !== EQUIVALENCE_FORMAT || d.pack !== pack || typeof d.equivalent !== "boolean") return { counted: false, reason: `reported only: ${file} is not a ${EQUIVALENCE_FORMAT} declaration of ${pack}` };
  if (!d.equivalent) return { counted: false, reason: `reported only: ${file} says the translations are not equivalent` };
  const different = typeof d.different === "object" && d.different !== null ? Object.keys(d.different).length : 0;
  return { counted: true, reason: `counted: ${file} says equivalent${different > 0 ? ` (${different} rule${different === 1 ? "" : "s"} declared different)` : ""}` };
}

/** The nearest-rank percentile `p` (0–100) of `values`. */
export function percentile(values: readonly number[], p: number): number {
  if (values.length === 0) throw new Error("percentile of no values");
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.max(0, Math.ceil((p / 100) * sorted.length) - 1))]!;
}

export interface RuleSliceStats {
  rules: number;
  /** Rules both sides timed. */
  compared: number;
  /** Over the first runs' ratios of the compared rules (finite ones). */
  medianRatio: number | null;
  p95Ratio: number | null;
  /** Over the bar at the first runs, before confirmation and the floor. */
  overBefore: string[];
  /** Still over after confirmation and moved by at least the floor: the rules that fail. */
  failing: string[];
  /** Final ratio above RULE_ALERT_RATIO and moved by at least the floor. */
  overAlert: string[];
  /** The worst final ratios, worst first, with each side's ms (the confirmation's where made). */
  worst: { rule: string; ratio: number; main: number; candidate: number }[];
}

/** Slice B's summary of one pack's rules alone: the ratio distribution, the rules over the bar and the worst ones. */
export function ruleSliceStats(rules: readonly TimeVerdict[], worstCount = 5, floorMs = RULE_FLOOR_MS): RuleSliceStats {
  const compared = rules.filter((r) => r.status !== "not-compared");
  const ratios = compared.map((r) => r.ratio!).filter(Number.isFinite);
  return {
    rules: rules.length,
    compared: compared.length,
    medianRatio: ratios.length > 0 ? median(ratios) : null,
    p95Ratio: ratios.length > 0 ? percentile(ratios, 95) : null,
    overBefore: compared.filter((r) => r.overBefore).map((r) => r.name),
    failing: compared.filter((r) => r.status === "fail").map((r) => r.name),
    overAlert: compared.filter((r) => (r.finalRatio ?? 0) > RULE_ALERT_RATIO && (r.finalDeltaMs ?? 0) >= floorMs).map((r) => r.name),
    worst: [...compared]
      .sort((a, b) => (b.finalRatio ?? 0) - (a.finalRatio ?? 0) || (b.finalDeltaMs ?? 0) - (a.finalDeltaMs ?? 0) || (a.name < b.name ? -1 : 1))
      .slice(0, worstCount)
      .map((r) => ({ rule: r.name, ratio: r.finalRatio!, main: r.confirm?.main ?? r.main!, candidate: r.confirm?.candidate ?? r.candidate! })),
  };
}

// ---------------------------------------------------------------- the phase that moved

export const PHASES = ["load", "compile", "evaluate", "startup"] as const;

/** The part of a run row a locator reads: product-run.ts's in-process phases beside the process's own figures. */
export interface PhaseRun {
  mode: "cold" | "warm";
  completed: boolean;
  wallMs: number;
  rssKB: number;
  phases?: { load_ms: number; compile_ms: number; evaluate_ms: number } | null;
}

/** Each side's median of every phase over its completed runs of one mode; startup is the wall outside the phases. */
function phaseMedians(runs: readonly PhaseRun[], mode: "cold" | "warm"): Record<(typeof PHASES)[number], number> | null {
  const timed = runs.filter((r) => r.completed && r.mode === mode && r.phases != null);
  if (timed.length === 0) return null;
  const of = (pick: (r: PhaseRun) => number): number => median(timed.map(pick));
  return {
    load: of((r) => r.phases!.load_ms),
    compile: of((r) => r.phases!.compile_ms),
    evaluate: of((r) => r.phases!.evaluate_ms),
    startup: of((r) => r.wallMs - r.phases!.load_ms - r.phases!.compile_ms - r.phases!.evaluate_ms),
  };
}

/**
 * One line naming the phase a failing figure moved in, from both sides' runs of its mode: the phase (load, compile,
 * evaluate, or startup: the process's wall outside them) whose median grew most, with every phase's change. A warm
 * figure over the bar whose cold counterpart passes moved in the store (`coldPasses`). Peak RSS names startup when the
 * empty rule set's peak grew by at least half as much (`fixed`), else the rules' phase whose time grew more (compile or
 * evaluate). `only` limits the phases considered (a rule alone: compile and evaluate).
 */
export function locate(metric: string, main: readonly PhaseRun[], candidate: readonly PhaseRun[], options: { coldPasses?: boolean; fixed?: { main: readonly PhaseRun[]; candidate: readonly PhaseRun[] }; only?: readonly (typeof PHASES)[number][] } = {}): string {
  const mode = metric.endsWith("warm") ? "warm" : "cold";
  const m = phaseMedians(main, mode);
  const c = phaseMedians(candidate, mode);
  const deltas = m === null || c === null ? null : (options.only ?? PHASES).map((phase) => ({ phase, delta: c[phase] - m[phase] }));
  const top = deltas === null ? null : deltas.reduce((best, d) => (d.delta > best.delta ? d : best));
  const detail = deltas === null ? "no phases recorded" : deltas.map((d) => `${d.phase} ${d.delta < 0 ? "−" : "+"}${Math.round(Math.abs(d.delta))} ms`).join(", ");
  if (metric.startsWith("rss")) {
    const rules = m !== null && c !== null && c.compile - m.compile > c.evaluate - m.evaluate ? "compile" : "evaluate";
    const rssOf = (runs: readonly PhaseRun[]): number[] => runs.filter((r) => r.completed && r.mode === mode).map((r) => r.rssKB);
    const item = [rssOf(main), rssOf(candidate)];
    const fixed = options.fixed === undefined ? null : [rssOf(options.fixed.main), rssOf(options.fixed.candidate)];
    if (fixed !== null && [...fixed, ...item].every((v) => v.length > 0)) {
      const itemDelta = median(item[1]!) - median(item[0]!);
      const fixedDelta = median(fixed[1]!) - median(fixed[0]!);
      const mb = (kb: number): string => `${kb < 0 ? "−" : "+"}${Math.round(Math.abs(kb) / 1024)} MB`;
      return `${fixedDelta >= itemDelta / 2 ? "startup" : rules}: the empty rule set's peak RSS ${mb(fixedDelta)} of ${mb(itemDelta)}; ${detail}`;
    }
    return `${rules}: peak RSS (no empty-rule-set runs to split it); ${detail}`;
  }
  if (options.coldPasses === true && mode === "warm") return `store: over only on the reused store (the cold run passes); ${detail}`;
  return `${top?.phase ?? "unknown"}: ${detail}`;
}

// ---------------------------------------------------------------- slice C: all rules × one file

/**
 * Each pack's source extensions: slice C picks its files among these in the pack's real-code corpus (a PHP file that
 * is a Blade template is the blade pack's). A pack main ships that is missing here cannot be measured.
 */
export const SOURCE_EXTENSIONS: Readonly<Record<string, readonly string[]>> = {
  blade: [".blade.php"],
  complexity: [".ts"],
  javascript: [".ts"],
  next: [".tsx", ".jsx", ".ts", ".js"],
  node: [".ts"],
  php: [".php"],
  python: [".py"],
  react: [".tsx", ".jsx", ".js", ".ts"],
  rerail: [".ts"],
  rust: [".rs"],
  sql: [".sql"],
  typescript: [".ts"],
  vue: [".vue"],
};

export type FileLabel = "largest" | "median" | "small";

/**
 * Slice C's three files of a corpus, by bytes among the pack's source files (ties by path): the largest, the median,
 * and a small one (the 5th percentile: small, not degenerate). Empty where the corpus has no source file of the pack.
 */
export function pickFiles(pack: string, files: readonly { path: string; bytes: number }[]): { label: FileLabel; path: string; bytes: number }[] {
  const extensions = SOURCE_EXTENSIONS[pack];
  if (extensions === undefined) throw new Error(`no source extensions declared for pack ${pack} (verdict.ts SOURCE_EXTENSIONS)`);
  const sources = files
    .filter((f) => extensions.some((ext) => f.path.endsWith(ext)) && !(pack === "php" && f.path.endsWith(".blade.php")))
    .sort((a, b) => a.bytes - b.bytes || (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  if (sources.length === 0) return [];
  const at = (label: FileLabel, i: number): { label: FileLabel; path: string; bytes: number } => ({ label, path: sources[i]!.path, bytes: sources[i]!.bytes });
  return [at("largest", sources.length - 1), at("median", sources.length >> 1), at("small", Math.floor(sources.length / 20))];
}
