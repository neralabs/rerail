import { engineJson } from "./engine-json.js";
// What a cell's time is made of, beside the gross figures the 1.10× bar judges (reported, never a status):
//
//   fixed    each side's start-up, T0: the product path with an empty rule set over the same corpus, cold and warm;
//   net      the run less its side's T0 (wall and CPU, cold and warm), and the candidate's net over the baseline's;
//   phases   product-run.ts's in-process load, compile and evaluate times; compile per rule, and evaluate (less the
//            empty set's) per rule per syntax node of the languages the rules read;
//   workload the corpus's files, bytes, lines and lexical tokens per language, and the engine's own node count
//            (rerail-engine run-rr with a one-line counting rule per language);
//   multiple each rule timed alone in the drill-down over the pack's median rule on that corpus, flagged above
//            MULTIPLE_FLAG, or when a rule at least as costly as the median grew more than MULTIPLE_GROWTH_FLAG times
//            (informational).
//
// Why: the gross ratio divides every regression by the ~0.65 s Node + tsx start-up, so a pack whose rules take 60 ms
// passes a doubling; and a per-rule constant (compile) looks like a per-file cost unless the phases are apart. See
// the proposal that chose these: ~/rerail-bench/final-shape/shipnoship/metric-proposal.md (not in this repository).

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { median } from "./verdict.js";

// ---------------------------------------------------------------- phases

/** product-run.ts's in-process phases of one run (ms). */
export interface Phases {
  load_ms: number;
  compile_ms: number;
  evaluate_ms: number;
}

/** The phases product-run.ts wrote to its result; null for any other result (run-policies, a failure). */
export function readPhases(output: { loadMs?: unknown; compileMs?: unknown; evaluateMs?: unknown }): Phases | null {
  const { loadMs, compileMs, evaluateMs } = output;
  if (typeof loadMs !== "number" || typeof compileMs !== "number" || typeof evaluateMs !== "number") return null;
  return { load_ms: Math.round(loadMs), compile_ms: Math.round(compileMs), evaluate_ms: Math.round(evaluateMs) };
}

// ---------------------------------------------------------------- workload

/**
 * File suffix to the language family the engine parses it as. One counting rule per family: `.language typescript`
 * parses .js/.jsx/.mjs/.cjs as well; .blade.php is blade, not php. JSON is listed (the product path reads it) but no
 * shipped rule parses it, so it has no node count.
 */
const SUFFIXES: readonly (readonly [string, string])[] = [
  [".blade.php", "blade"], [".ts", "typescript"], [".tsx", "typescript"], [".mts", "typescript"], [".cts", "typescript"],
  [".js", "typescript"], [".jsx", "typescript"], [".mjs", "typescript"], [".cjs", "typescript"], [".py", "python"],
  [".pyi", "python"], [".rs", "rust"], [".php", "php"], [".sql", "sql"], [".vue", "vue"], [".json", "json"],
];
/** A rule's `language` as the family whose nodes it reads. */
const FAMILY: Readonly<Record<string, string>> = { javascript: "typescript", tsx: "typescript", typescript: "typescript", python: "python", rust: "rust", php: "php", blade: "blade", sql: "sql", vue: "vue" };
const PARSED = new Set(Object.values(FAMILY));

/** The counts' known gaps, stated in the report beside them. */
export const KNOWN_GAPS = [
  "syntax node counts cover each file's host language only: Vue's `<script>` and Blade's PHP code are not counted (Vue has 0.3–0.4 nodes per lexical token against about 1 elsewhere), so evaluate per rule·node overstates those packs",
  "node counts come from the candidate checkout's rerail-engine; both sides are divided by the same count",
  "start-up (T0) and phases are the product path's; the .rr side has neither (run-policies takes no empty rule set)",
];

export interface LanguageWorkload {
  files: number;
  bytes: number;
  lines: number;
  tokens: number;
  /** The engine's node count (named nodes and comments); null where the language is not parsed or counting failed. */
  nodes: number | null;
}

export interface Workload {
  files: number;
  bytes: number;
  lines: number;
  tokens: number;
  /** Over the parsed languages only; null when no node count was made (no engine binary). */
  nodes: number | null;
  by_language: Record<string, LanguageWorkload>;
  counted_by: string | null;
}

// One generic lexer for every language: identifiers, numbers, string literals, and each other non-space character.
const TOKEN = /[A-Za-z_$][\w$]*|\d[\w.]*|"(?:\\.|[^"\\\n])*"|'(?:\\.|[^'\\\n])*'|`[^`]*`|\S/g;

/** A file's lines (a last line without a newline counts) and lexical tokens. */
export function lexicalCounts(text: string): { lines: number; tokens: number } {
  const newlines = text.split("\n").length - 1;
  return { lines: newlines + (text === "" || text.endsWith("\n") ? 0 : 1), tokens: text.match(TOKEN)?.length ?? 0 };
}

export function languageOf(path: string): string | undefined {
  return SUFFIXES.find(([suffix]) => path.endsWith(suffix))?.[1];
}

function walk(root: string, visit: (path: string) => void): void {
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const full = join(root, entry.name);
    if (entry.isDirectory()) walk(full, visit);
    else if (entry.isFile()) visit(full);
  }
}

/** A built rerail-engine binary in a checkout, if any (`.exe` on Windows): the harness's one way to find it. */
export function engineBinary(checkout: string): string | undefined {
  const name = process.platform === "win32" ? "rerail-engine.exe" : "rerail-engine";
  const candidates = [join(checkout, "target", "release", name), join(checkout, "native", "target", "release", name)];
  const target = join(checkout, "target");
  if (existsSync(target)) for (const triple of readdirSync(target)) candidates.push(join(target, triple, "release", name));
  return candidates.find((path) => existsSync(path));
}

/**
 * The engine's node count of a corpus's files of one family: its run's `nodes` (null when the run fails). A count, not
 * a timing, so the run is not pinned: it runs alike on every platform.
 */
function countNodes(engine: string, corpus: string, family: string): number | null {
  const scratch = mkdtempSync(join(tmpdir(), "shipbar-nodes-"));
  try {
    const rule = join(scratch, `nodes-${family}.rr`);
    writeFileSync(
      rule,
      [
        `.policy "shipbar-nodes-${family}" revision 1`,
        `.description "Each document's syntax node count, one finding per document."`,
        `.language ${family}`,
        ".mode steer",
        '.scope include "**/*"',
        "",
        '.violation nodes(at: node, msg: symbol) code "NODES" "{msg}"',
        "nodes(r, to_string(k)) :- kind(r, _), !child(_, r, _), docof(r, d), k = count : { docof(n, d), kind(n, _) }.",
        "",
      ].join("\n"),
    );
    mkdirSync(join(scratch, "tmp"));
    const env = { ...process.env, RERAIL_TAPE_CACHE: join(scratch, "store.sqlite"), TMPDIR: join(scratch, "tmp") };
    const r = spawnSync(engine, ["run-rr", rule, corpus, join(scratch, "out")], { env, encoding: "utf8", maxBuffer: 1 << 30 });
    if (r.status !== 0) return null;
    const result = engineJson(r.stdout);
    return result !== null && typeof result === "object" && "nodes" in result && typeof result.nodes === "number" ? result.nodes : null;
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

/** A corpus's workload; node counts need `engine` (left null without one). */
export function measureWorkload(corpus: string, engine: string | undefined): Workload {
  const by: Record<string, LanguageWorkload> = {};
  walk(corpus, (path) => {
    const language = languageOf(path);
    if (language === undefined) return;
    const text = readFileSync(path, "utf8");
    const counts = lexicalCounts(text);
    const w = (by[language] ??= { files: 0, bytes: 0, lines: 0, tokens: 0, nodes: null });
    w.files += 1;
    w.bytes += Buffer.byteLength(text);
    w.lines += counts.lines;
    w.tokens += counts.tokens;
  });
  if (engine !== undefined) for (const [language, w] of Object.entries(by)) if (PARSED.has(language)) w.nodes = countNodes(engine, corpus, language);
  const all = Object.values(by);
  const parsed = Object.entries(by).filter(([language]) => PARSED.has(language)).map(([, w]) => w.nodes);
  const sum = (key: "files" | "bytes" | "lines" | "tokens"): number => all.reduce((s, w) => s + w[key], 0);
  return {
    files: sum("files"),
    bytes: sum("bytes"),
    lines: sum("lines"),
    tokens: sum("tokens"),
    nodes: engine === undefined || parsed.some((n) => n === null) ? null : parsed.reduce<number>((s, n) => s + n!, 0),
    by_language: by,
    counted_by: engine ?? null,
  };
}

/** The file a corpus's workload is cached in: its key's path parts joined, alike for `/` and Windows' `\`. */
function workloadFile(out: string, key: string): string {
  return join(out, "workload", `${key.split(/[\\/]/).join("_")}.json`);
}

/** A corpus's workload, counted once per run directory (<out>/workload/<key>.json) and read back after. */
export function cachedWorkload(out: string, key: string, corpus: string, engine: string | undefined): Workload {
  const path = workloadFile(out, key);
  if (existsSync(path)) return JSON.parse(readFileSync(path, "utf8")) as Workload;
  const workload = measureWorkload(corpus, engine);
  mkdirSync(join(out, "workload"), { recursive: true });
  writeFileSync(path, JSON.stringify(workload, null, 1));
  return workload;
}

export function readWorkload(out: string, key: string): Workload | null {
  const path = workloadFile(out, key);
  return existsSync(path) ? (JSON.parse(readFileSync(path, "utf8")) as Workload) : null;
}

// ---------------------------------------------------------------- rule sets

/** A rule set's size: its rules, and per language family how many read it (a rule reading k families counts 1/k each). */
export interface RuleSet {
  rules: number;
  by_language: Record<string, number>;
}

/** The families a shipped JSON rule reads, from its `"language"` fields; none for a text rule. */
export function ruleFamilies(json: string): string[] {
  return [...new Set([...json.matchAll(/"language"\s*:\s*"(\w+)"/g)].flatMap((m) => (FAMILY[m[1]!] === undefined ? [] : [FAMILY[m[1]!]!])))].sort();
}

/** A pack's rules directory as a rule set, optionally only the given rule ids. */
export function ruleSet(rulesDir: string, only?: ReadonlySet<string>): RuleSet {
  const set: RuleSet = { rules: 0, by_language: {} };
  if (!existsSync(rulesDir)) return set;
  for (const name of readdirSync(rulesDir).filter((n) => n.endsWith(".json"))) {
    if (only !== undefined && !only.has(name.slice(0, -".json".length))) continue;
    set.rules += 1;
    const families = ruleFamilies(readFileSync(join(rulesDir, name), "utf8"));
    for (const family of families.length === 0 ? ["*"] : families) set.by_language[family] = (set.by_language[family] ?? 0) + 1 / Math.max(families.length, 1);
  }
  return set;
}

/** Σ over languages of rules × that language's nodes (a text rule, `*`, reads every parsed node); null without counts. */
export function ruleNodes(rules: RuleSet, workload: Workload | null): number | null {
  if (workload === null || workload.nodes === null) return null;
  let total = 0;
  for (const [family, count] of Object.entries(rules.by_language)) {
    const nodes = family === "*" ? workload.nodes : (workload.by_language[family]?.nodes ?? 0);
    total += count * nodes;
  }
  return total;
}

// ---------------------------------------------------------------- a cell's cost

/** The part of a run row the cost reads. */
export interface CostRun {
  mode: "cold" | "warm";
  completed: boolean;
  wallMs: number;
  cpuMs: number;
  phases?: Phases | null;
}

export const NET_METRICS = ["wall-cold", "wall-warm", "cpu-cold", "cpu-warm"] as const;
type NetMetric = (typeof NET_METRICS)[number];

export interface SideCost {
  /** T0: the empty rule set's medians on this corpus (ms); null where not measured. */
  fixed: Record<NetMetric, number | null> | null;
  /** Cold runs' median phases (ms); null where the side's path has none. */
  phases: Phases | null;
  rules: RuleSet;
  /** Σ rules × nodes of the languages they read. */
  rule_nodes: number | null;
  compile_ms_per_rule: number | null;
  /** Cold evaluate less the empty rule set's, in ns per rule per node. */
  evaluate_ns_per_rule_node: number | null;
}

export interface NetItem {
  metric: NetMetric;
  /** Median run less median T0 (ms); null where either is missing. */
  baseline_ms: number | null;
  candidate_ms: number | null;
  /** candidate_ms / baseline_ms; null when the baseline's net is not positive. */
  ratio: number | null;
}

export interface CellCost {
  baseline: SideCost;
  candidate: SideCost;
  net: NetItem[];
  workload: Workload | null;
}

function medianOf(values: readonly number[]): number | null {
  return values.length > 0 ? median(values) : null;
}

function metricMedian(runs: readonly CostRun[], metric: NetMetric): number | null {
  const mode = metric.endsWith("cold") ? "cold" : "warm";
  return medianOf(runs.filter((r) => r.completed && r.mode === mode).map((r) => (metric.startsWith("wall") ? r.wallMs : r.cpuMs)));
}

function phaseMedians(runs: readonly CostRun[]): Phases | null {
  const cold = runs.filter((r) => r.completed && r.mode === "cold" && r.phases != null).map((r) => r.phases!);
  if (cold.length === 0) return null;
  return { load_ms: median(cold.map((p) => p.load_ms)), compile_ms: median(cold.map((p) => p.compile_ms)), evaluate_ms: median(cold.map((p) => p.evaluate_ms)) };
}

function sideCost(runs: readonly CostRun[], fixedRuns: readonly CostRun[], rules: RuleSet, workload: Workload | null): SideCost {
  const fixed = fixedRuns.some((r) => r.completed) ? (Object.fromEntries(NET_METRICS.map((m) => [m, metricMedian(fixedRuns, m)])) as Record<NetMetric, number | null>) : null;
  const phases = phaseMedians(runs);
  const empty = phaseMedians(fixedRuns);
  const ruleNodeCount = ruleNodes(rules, workload);
  return {
    fixed,
    phases,
    rules,
    rule_nodes: ruleNodeCount,
    compile_ms_per_rule: phases === null || rules.rules === 0 ? null : phases.compile_ms / rules.rules,
    evaluate_ns_per_rule_node: phases === null || ruleNodeCount === null || ruleNodeCount === 0 ? null : ((phases.evaluate_ms - (empty?.evaluate_ms ?? 0)) * 1e6) / ruleNodeCount,
  };
}

/** A cell's cost from both sides' runs, their empty-rule-set runs, their rule sets and the corpus's workload. */
export function cellCost(input: { baseline: { runs: readonly CostRun[]; fixed: readonly CostRun[]; rules: RuleSet }; candidate: { runs: readonly CostRun[]; fixed: readonly CostRun[]; rules: RuleSet }; workload: Workload | null }): CellCost {
  const baseline = sideCost(input.baseline.runs, input.baseline.fixed, input.baseline.rules, input.workload);
  const candidate = sideCost(input.candidate.runs, input.candidate.fixed, input.candidate.rules, input.workload);
  const net = (runs: readonly CostRun[], side: SideCost, metric: NetMetric): number | null => {
    const total = metricMedian(runs, metric);
    const t0 = side.fixed?.[metric] ?? null;
    return total === null || t0 === null ? null : total - t0;
  };
  return {
    baseline,
    candidate,
    net: NET_METRICS.map((metric) => {
      const b = net(input.baseline.runs, baseline, metric);
      const c = net(input.candidate.runs, candidate, metric);
      return { metric, baseline_ms: b, candidate_ms: c, ratio: b === null || c === null || b <= 0 ? null : c / b };
    }),
    workload: input.workload,
  };
}

// ---------------------------------------------------------------- the drill-down's rule cost multiple

/** A rule above this many times the pack's median rule is flagged. */
export const MULTIPLE_FLAG = 10;
/** A rule whose multiple grew more than this many times from the baseline's is flagged. */
export const MULTIPLE_GROWTH_FLAG = 1.5;
/** Growth is flagged only for a rule at least as costly as the median rule: below it, a jump is milliseconds. */
export const GROWTH_FLOOR = 1;

export interface RuleMultiple {
  multiple: { baseline: number | null; candidate: number | null };
  /** Informational, never a status. */
  flags: string[];
}

/**
 * Each rule's time alone less the empty rule set's, over the median rule's of the same side: the multiple carries
 * from fixtures to real code where time per node does not (proposal, T7).
 */
export function ruleMultiples(rules: readonly { rule: string; baseline_ms: number | null; candidate_ms: number | null }[], startup: { baseline_ms: number | null; candidate_ms: number | null }): Map<string, RuleMultiple> {
  const own = (ms: number | null, start: number | null): number | null => (ms === null ? null : ms - (start ?? 0));
  const scale = (side: "baseline" | "candidate"): number | null => {
    const values = rules.flatMap((r) => {
      const v = own(r[`${side}_ms`], startup[`${side}_ms`]);
      return v === null ? [] : [v];
    });
    const m = medianOf(values);
    return m === null || m <= 0 ? null : m;
  };
  const scales = { baseline: scale("baseline"), candidate: scale("candidate") };
  const out = new Map<string, RuleMultiple>();
  for (const r of rules) {
    const of = (side: "baseline" | "candidate"): number | null => {
      const v = own(r[`${side}_ms`], startup[`${side}_ms`]);
      return v === null || scales[side] === null ? null : v / scales[side]!;
    };
    const multiple = { baseline: of("baseline"), candidate: of("candidate") };
    const flags: string[] = [];
    if (multiple.candidate !== null && multiple.candidate > MULTIPLE_FLAG) flags.push(`${multiple.candidate.toFixed(1)}× the pack's median rule`);
    if (multiple.candidate !== null && multiple.candidate >= GROWTH_FLOOR && multiple.baseline !== null && multiple.baseline > 0 && multiple.candidate / multiple.baseline > MULTIPLE_GROWTH_FLAG) {
      flags.push(`multiple grew ${multiple.baseline.toFixed(1)}× → ${multiple.candidate.toFixed(1)}×`);
    }
    out.set(r.rule, { multiple, flags });
  }
  return out;
}
