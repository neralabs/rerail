// `just rerail-sql-loop`: the SQL engine's validation loop over a checked-in subset of the corpus (`loop-subset.json`).
// The reporting clauses run in units: a policy's clauses in chunks, with its `define`s, each unit one workspace over the
// subset (one parse of each file it accepts), on SQL and on native, spread over `--jobs` long-lived child processes.
// The chunking is decided up front from the previous runs' recorded costs: a clause that timed out or reached
// `ISOLATE_MS` runs alone, the others are packed per policy up to `--chunk` CPU seconds; the units run longest first
// (LPT). A unit past its CPU cap (`--cap` alone, `--group-cap` a chunk) is killed: a TIMEOUT row with its CPU and wall
// time, never split and retried within the run. The verdict is parity (each clause's SQL lines against native's over
// the subset, apart from native's budget failures), speed (SQL/native CPU and wall over the units both finished, the
// costliest statements, clauses and (clause, file)s, against the previous run), a projection to the whole tree, and
// the whole-program SQL setup (every clause in one program, compiled, and a first session), capped. Native's lines over
// the subset and its whole-tree time are cached per native source hash (`~/rerail-bench/data-layer/loop-cache/`);
// every run is recorded in `~/rerail-bench/data-layer/loop-runs/`.
//
//   loop.ts run        [--rounds N] [--jobs N] [--memory GiB] [--chunk S] [--replan] [--cap S] [--group-cap S] [--clauses a,b] [--program-cap S] [--show N]
//   loop.ts calibrate  [--jobs N] [--cap S] [--group-cap S]: the latest run's units over the full corpus, against its projection
//   loop.ts costs      [--jobs N] [--cap S] [--files-from list]: each file's SQL time with every clause in one program, per-(clause, file) attribution
//   loop.ts select     --from <distilled subset.json>: writes loop-subset.json (the files, why, and per-clause native coverage)
//   loop.ts unit-child | program-child (internal): one JSON line per event on stdout
//
// Caps are CPU seconds (user + system, every thread of the child), so they do not move with the machine's load.
import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, writeFileSync, writeSync } from "node:fs";
import { cpus, homedir, loadavg, totalmem } from "node:os";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { collectRepository, modulePath } from "./corpus.js";
import {
  compareStrings, engineReportSchema, failureLine, loadEngine, loadRules, rerailRoot, RuleWorkspace, sha256, violationLine,
  type Engine, type EngineResult, type Matcher, type PortedRule, type SourceFile,
} from "./engine.js";

const script = fileURLToPath(import.meta.url);
const benchHome = resolve(homedir(), "rerail-bench");
const stateRoot = resolve(benchHome, "data-layer");
const defaultSubset = resolve(rerailRoot, "benchmark/data-layer/loop-subset.json");
const defaultCosts = resolve(stateRoot, "loop-costs.json");
const platform = `${process.platform}-${process.arch}`;
const defaultAddon = resolve(rerailRoot, `target-loop/addon/rerail_engine.${platform}.node`);
/** The SQL engine leaves native's work-budget failures alone: those (rule, clause, file)s are not compared. */
const BUDGET_FAILURE = "budget-exceeded";
/** A run is untrusted when the 1-minute load average before it exceeds this, or its rounds spread more than this. */
const LOAD_LIMIT = 6;
const SPREAD_LIMIT = 0.1;
/** A clause whose SQL CPU in the previous runs reached this runs alone, so it cannot sink a chunk. */
const ISOLATE_MS = 5000;
/** A clause no previous run measured: its SQL CPU estimate. */
const UNKNOWN_CLAUSE_MS = 250;
/** Previous runs a clause's cost is looked up in, newest first. */
const COST_HISTORY = 12;
/** A child is killed as stuck when its wall time passes this many times its CPU cap (plus a minute) without the cap. */
const WALL_GUARD = 6;
/** A unit no previous run measured on an engine, and the whole-program setup: their estimated peak resident sets. */
const UNKNOWN_UNIT_RSS_BYTES = 4 * 2 ** 30;
const UNKNOWN_PROGRAM_RSS_BYTES = 32 * 2 ** 30;
/** Statements and (clause, file)s a unit reports: its costliest. */
const UNIT_STATEMENTS = 30;
const UNIT_CLAUSE_FILES = 40;

class UsageError extends Error {}

type EngineName = "native" | "sql";

// ---------------------------------------------------------------------------------------------------------------------
// Options

interface Options {
  command: "run" | "calibrate" | "costs" | "select" | "unit-child" | "program-child";
  addon: string;
  corpus: string;
  policies: string;
  subset: string;
  rounds: number;
  jobs: number;
  /** A chunk of a policy's clauses: its target SQL CPU in seconds, from the previous runs' costs. */
  chunkSeconds: number;
  /** A clause alone: its CPU cap in seconds (costs: per file). */
  capSeconds: number;
  /** A chunk of clauses: its CPU cap in seconds; past it, the chunk is one TIMEOUT row (its clauses run alone next run). */
  groupCapSeconds: number;
  /** The whole-program SQL setup's CPU cap in seconds; 0 skips it. */
  programCapSeconds: number;
  /** The running units' estimated peak resident sets stay within this many GiB. */
  memoryGib: number;
  /** run: pack every policy afresh from the costs instead of keeping the previous run's units. */
  replan: boolean;
  /** Clause filters: a clause runs where its `linter/clause` name holds any of them. */
  clauses: string[];
  show: number;
  /** select: the distilled subset manifest. */
  from: string | undefined;
  filesFrom: string | undefined;
  output: string | undefined;
  // unit children only
  scope: "subset" | "full" | "list";
}

const valued = [
  "--addon", "--corpus", "--policies", "--subset", "--rounds", "--jobs", "--chunk", "--cap", "--group-cap", "--program-cap",
  "--memory", "--clauses", "--show", "--from", "--files-from", "--output", "--scope",
];
const commands = ["run", "calibrate", "costs", "select", "unit-child", "program-child"] as const;

function integer(name: string, value: string, minimum: number): number {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < minimum) throw new UsageError(`${name} takes an integer ≥ ${minimum}, not ${value}`);
  return parsed;
}

function parseOptions(argv: readonly string[]): Options {
  const [first, ...rest] = argv;
  const command = first === undefined || first.startsWith("--") ? "run" : first;
  const args = command === first ? rest : argv;
  if (!commands.includes(command as Options["command"])) throw new UsageError(`unknown command ${command}`);
  const values: Record<string, string> = {};
  let replan = false;
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]!;
    if (argument === "--replan") {
      replan = true;
      continue;
    }
    const [key, inline] = argument.includes("=") ? [argument.slice(0, argument.indexOf("=")), argument.slice(argument.indexOf("=") + 1)] : [argument, undefined];
    if (!valued.includes(key)) throw new UsageError(`unknown option ${argument}`);
    const value = inline ?? args[++index];
    if (value === undefined) throw new UsageError(`${key} needs a value`);
    values[key] = value;
  }
  const scope = values["--scope"] ?? "subset";
  if (scope !== "subset" && scope !== "full" && scope !== "list") throw new UsageError(`--scope takes subset, full or list, not ${scope}`);
  const defaultCap = command === "calibrate" ? "300" : command === "costs" ? "15" : "60";
  return {
    replan,
    command: command as Options["command"],
    addon: resolve(values["--addon"] ?? process.env.RERAIL_LOOP_ADDON ?? defaultAddon),
    corpus: resolve(values["--corpus"] ?? process.env.RERAIL_DATA_LAYER_CORPUS ?? resolve(benchHome, "nera-data-layer-fce4dc72")),
    policies: resolve(values["--policies"] ?? process.env.RERAIL_DATA_LAYER_POLICIES ?? resolve(stateRoot, "policies")),
    subset: resolve(values["--subset"] ?? defaultSubset),
    rounds: integer("--rounds", values["--rounds"] ?? "1", 1),
    jobs: integer("--jobs", values["--jobs"] ?? "14", 1),
    chunkSeconds: integer("--chunk", values["--chunk"] ?? "10", 1),
    capSeconds: integer("--cap", values["--cap"] ?? defaultCap, 1),
    groupCapSeconds: integer("--group-cap", values["--group-cap"] ?? (command === "calibrate" ? "600" : "90"), 1),
    programCapSeconds: integer("--program-cap", values["--program-cap"] ?? "60", 0),
    memoryGib: integer("--memory", values["--memory"] ?? String(Math.floor(totalmem() / 2 ** 30 / 2)), 1),
    clauses: (values["--clauses"] ?? "").split(",").filter(filter => filter.length > 0),
    show: integer("--show", values["--show"] ?? "12", 0),
    from: values["--from"] === undefined ? undefined : resolve(values["--from"]),
    filesFrom: values["--files-from"] === undefined ? undefined : resolve(values["--files-from"]),
    output: values["--output"] === undefined ? undefined : resolve(values["--output"]),
    scope,
  };
}

// ---------------------------------------------------------------------------------------------------------------------
// The subset manifest

const clauseCoverageSchema = z.object({
  /** A subset file native reports a finding of the clause on (over the subset alone); null when none. */
  finding: z.string().nullable(),
  /** A subset file in the rule's scope where the clause finds nothing. */
  clean: z.string().nullable(),
  /** Native findings of the clause over the subset alone, and over the full tree. */
  subsetFindings: z.number().int(),
  fullFindings: z.number().int(),
  /** `select_when`, `require_each` or `budget`: its findings depend on other files. */
  crossFile: z.boolean(),
  status: z.enum(["covered", "no-finding-in-full-tree", "finding-only-in-full-run", "no-clean-file"]),
});
const calibrationSchema = z.looseObject({
  date: z.string(),
  /** The addon (sha256) and HEAD the calibration measured. */
  addon: z.string(),
  head: z.string(),
  /** Full-corpus SQL/native CPU ratio ÷ the subset's, over the units both finished: what a run's ratio is multiplied by. */
  correction: z.number(),
  /** The corrected projection's error: per policy, its correction taken from the other policies; Σ|error| ÷ Σ actual. */
  error: z.number(),
  /** The uncorrected projection's error: the subset ratio ÷ the full-corpus ratio − 1. */
  totalError: z.number(),
  units: z.number().int(),
  /** Units past their cap on the full corpus: not calibrated. */
  timeouts: z.array(z.string()),
  note: z.string(),
});
const subsetSchema = z.object({
  format: z.literal("rerail.sql-loop-subset/v3"),
  generated: z.string(),
  corpus: z.object({ commit: z.string(), files: z.number().int(), bytes: z.number().int() }),
  policies: z.record(z.string(), z.string()),
  /** The distilled subset the files come from (`select --from`). */
  source: z.object({ path: z.string(), sha256: z.string(), sources: z.array(z.string()) }),
  files: z.array(z.object({ path: z.string(), bytes: z.number().int(), reasons: z.array(z.string()) })),
  clauses: z.record(z.string(), clauseCoverageSchema),
  calibration: calibrationSchema.nullable(),
});
type Subset = z.infer<typeof subsetSchema>;
type Calibration = z.infer<typeof calibrationSchema>;

function readSubset(path: string): Subset {
  if (!existsSync(path)) throw new UsageError(`${path} does not exist: run \`loop.ts select\` first`);
  return subsetSchema.parse(JSON.parse(readFileSync(path, "utf8")));
}

const subsetSha = (subset: Subset) => sha256(JSON.stringify(subset.files.map(file => file.path)));

/** Files with their working-tree contents and module paths, as `corpus.ts` collects them. */
function readFiles(engine: Engine, corpus: string, paths: readonly string[]): SourceFile[] {
  const root = realpathSync(corpus);
  return paths.map(path => ({ path, language: "python" as const, source: readFileSync(resolve(root, path), "utf8"), ...modulePath(engine, root, path) }));
}

function git(root: string, ...args: string[]): string {
  const result = spawnSync("git", ["-C", root, ...args], { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 });
  if (result.status !== 0) throw new Error(`git -C ${root} ${args.join(" ")} failed: ${result.stderr.trim()}`);
  return result.stdout;
}

// ---------------------------------------------------------------------------------------------------------------------
// Clauses and units

const documentSchema = z.looseObject({
  id: z.string(),
  clauses: z.array(z.looseObject({
    id: z.string(), define: z.unknown().optional(), require_each: z.unknown().optional(), budget: z.unknown().optional(),
    forbid: z.record(z.string(), z.unknown()).optional(),
  })),
});

interface Clause {
  /** `linter/clause`: the census's and the corpus timings' name. */
  name: string;
  linter: string;
  clauseId: string;
  /** `select_when`, `require_each` or `budget`: its findings depend on other files. */
  crossFile: boolean;
}

function reportingClauses(rules: readonly PortedRule[], filters: readonly string[] = []): Clause[] {
  return rules.flatMap(rule => documentSchema.parse(rule.document).clauses
    .filter(clause => clause.define === undefined)
    .map(clause => ({
      name: `${rule.linter}/${clause.id}`,
      linter: rule.linter,
      clauseId: clause.id,
      crossFile: clause.require_each !== undefined || clause.budget !== undefined || clause.forbid?.select_when !== undefined,
    }))
    .filter(clause => filters.length === 0 || filters.some(filter => clause.name.includes(filter))));
}

/** One workspace's clauses: some reporting clauses of one policy (`linter/clause` names), with its `define`s. */
const unitSchema = z.object({ linter: z.string(), clauses: z.array(z.string()) });
type Unit = z.infer<typeof unitSchema>;

/** The unit of every clause of every policy in one workspace (`costs`): production's shape. */
const ALL = "*";

const unitKey = (unit: Unit) => (unit.linter === ALL ? "every clause" : unit.clauses.length === 1 ? unit.clauses[0]! : `${unit.linter}[${unit.clauses.length}: ${unit.clauses[0]!.slice(unit.linter.length + 1)} …]`);

/** `<policies>/<linter>/policy.json` with only `unit`'s clauses and the policy's `define`s, compiled. */
function unitRule(engine: Engine, policies: string, unit: Unit): PortedRule {
  const path = resolve(policies, unit.linter, "policy.json");
  if (!existsSync(path)) throw new Error(`no policy for ${unit.linter}: ${path}`);
  // From the file: `PortedRule.document` keeps only a policy's id and clauses.
  const document = documentSchema.parse(JSON.parse(readFileSync(path, "utf8")));
  const ids = new Set(unit.clauses.map(name => name.slice(unit.linter.length + 1)));
  const clauses = document.clauses.filter(clause => clause.define !== undefined || ids.has(clause.id));
  const source = JSON.stringify({ ...document, clauses });
  return { linter: unit.linter, path, sha256: sha256(source), clauses: clauses.length, document: { ...document, clauses }, compiled: engine.binding.CodeRuleDslRuntime.compile(source, "json", engine.capabilities.source) };
}

function linesOf(result: EngineResult): string[] {
  return [...result.violations.map(violationLine), ...result.failures.map(failureLine)].sort(compareStrings);
}

// ---------------------------------------------------------------------------------------------------------------------
// Parity

interface Line {
  text: string;
  kind: string;
  rule: string;
  clause: string;
  code: string;
  path: string;
}

function parseLine(text: string): Line {
  const [kind = "", rule = "", clause = "", code = "", path = ""] = text.split("\t");
  return { text, kind, rule, clause, code, path };
}

interface ClauseParity {
  clause: string;
  compared: number;
  /** (rule, clause, file)s native ran out of its work budget on: not compared. */
  budgetExcluded: string[];
  files: { path: string; onlyNative: string[]; onlySql: string[] }[];
}

/** One clause's SQL lines against native's, apart from the files where native ran out of its work budget. */
function compareLines(clause: string, native: readonly string[], sql: readonly string[]): ClauseParity {
  const key = (line: Line) => `${line.rule}\t${line.clause}\t${line.path}`;
  const parsedNative = native.map(parseLine);
  const excluded = new Set(parsedNative.filter(line => line.kind === "failure" && line.code === BUDGET_FAILURE).map(key));
  const left = parsedNative.filter(line => !excluded.has(key(line)));
  const right = sql.map(parseLine).filter(line => !excluded.has(key(line)));
  const count = (lines: Line[]) => {
    const counts = new Map<string, number>();
    for (const line of lines) counts.set(line.text, (counts.get(line.text) ?? 0) + 1);
    return counts;
  };
  const leftCounts = count(left);
  const rightCounts = count(right);
  const files = new Map<string, ClauseParity["files"][number]>();
  const file = (line: Line) => {
    let found = files.get(line.path);
    if (found === undefined) files.set(line.path, found = { path: line.path, onlyNative: [], onlySql: [] });
    return found;
  };
  for (const line of left) if ((rightCounts.get(line.text) ?? 0) < (leftCounts.get(line.text) ?? 0)) file(line).onlyNative.push(line.text);
  for (const line of right) if ((leftCounts.get(line.text) ?? 0) < (rightCounts.get(line.text) ?? 0)) file(line).onlySql.push(line.text);
  for (const found of files.values()) {
    found.onlyNative = [...new Set(found.onlyNative)];
    found.onlySql = [...new Set(found.onlySql)];
  }
  return { clause, compared: left.length, budgetExcluded: [...excluded].sort(compareStrings), files: [...files.values()].sort((a, b) => compareStrings(a.path, b.path)) };
}

// ---------------------------------------------------------------------------------------------------------------------
// Unit children: one unit at a time, on requests from stdin

const statementSchema = z.object({ sql: z.string(), milliseconds: z.number(), rows: z.number(), runs: z.number() });
const requestSchema = z.object({
  id: z.number().int(),
  engine: z.enum(["native", "sql"]),
  unit: unitSchema,
  /** Only these files of the child's scope (those the unit's rule accepts). */
  files: z.array(z.string()).optional(),
  /** One file per apply, each timed (`costs`); SQL then runs batches of one file. */
  perFile: z.boolean().default(false),
  /** The unit's CPU cap (per file: the setup before the first file); past it the child reports `timeout` and dies. */
  capMs: z.number(),
  /** Per file: each file's CPU cap. */
  fileCapMs: z.number().optional(),
});
type UnitRequest = z.input<typeof requestSchema>;
const clauseResultSchema = z.object({
  clause: z.string(),
  lines: z.array(z.string()),
  /** SQL: the clause's statement time in the profile (its share of the unit's time); null on native. */
  sqlMs: z.number().nullable(),
});
/**
 * Where a unit's CPU went, as far as the harness and `engineReport()` can tell. `programCpuMs`: SQL lowering and
 * emission (the first engine report builds the program; timed apart, counted in the unit's time); `applyCpuMs`: the
 * apply (parse, plans, sessions, statements, rendering); the rest is the SQL engine's own timers (wall, summed over
 * its worker threads), null on native. Parse and per-file plan building have no timer in either engine.
 */
const phasesSchema = z.object({
  ruleCompileMs: z.number(),
  programCpuMs: z.number(),
  applyCpuMs: z.number(),
  sessionSetupMs: z.number().nullable(),
  statementMs: z.number().nullable(),
  stageMs: z.number().nullable(),
  selectMs: z.number().nullable(),
  inputMs: z.number().nullable(),
  factsMs: z.number().nullable(),
  sessions: z.number().nullable(),
  statementsRun: z.number().nullable(),
  statementsSkipped: z.number().nullable(),
});
type Phases = z.infer<typeof phasesSchema>;

const unitEventSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("ready"), setupMs: z.number(), files: z.number().int(), addon: z.object({ path: z.string(), sha256: z.string(), kernelVersion: z.string(), work: z.string() }), workerThreads: z.number() }),
  /** The request was read: its accepted files. */
  z.object({ kind: z.literal("start"), id: z.number().int(), files: z.array(z.string()) }),
  z.object({ kind: z.literal("file-start"), id: z.number().int(), path: z.string() }),
  /** A file of a per-file unit done; on SQL, the statement time the profile gives each clause on it. */
  z.object({ kind: z.literal("file"), id: z.number().int(), path: z.string(), wallMs: z.number(), cpuMs: z.number(), clauses: z.array(z.object({ clause: z.string(), milliseconds: z.number() })) }),
  z.object({
    kind: z.literal("unit"),
    id: z.number().int(),
    /** Compiling the unit's rule (the SQL program compiles and each worker's session opens inside the timed apply, as in the whole-tree run, where they open once for every clause). */
    compileMs: z.number(),
    wallMs: z.number(),
    cpuMs: z.number(),
    load: z.number(),
    files: z.number().int(),
    clauses: z.array(clauseResultSchema),
    refused: z.array(z.object({ clause: z.string(), reason: z.string() })),
    statements: z.array(statementSchema),
    clauseFiles: z.array(z.object({ clause: z.string(), path: z.string(), milliseconds: z.number() })),
    /** The child's peak resident set while it ran the unit (sampled every 20 ms). */
    rssBytes: z.number(),
    phases: phasesSchema,
    /** SQL: the compiled program's census (`engineReport().program`). */
    program: z.record(z.string(), z.number()).nullable(),
  }),
  /** Past its CPU cap (per file: on `path`): the child dies right after this event. */
  z.object({ kind: z.literal("timeout"), id: z.number().int(), cpuMs: z.number(), wallMs: z.number(), rssBytes: z.number(), path: z.string().nullable() }),
  /** The engine failed the unit (its rule did not compile, or an apply threw): the child goes on with the next request. */
  z.object({ kind: z.literal("error"), id: z.number().int(), message: z.string(), cpuMs: z.number(), rssBytes: z.number() }),
]);
type UnitEvent = z.infer<typeof unitEventSchema>;
type UnitDone = Extract<UnitEvent, { kind: "unit" }>;
type ReadyEvent = Extract<UnitEvent, { kind: "ready" }>;

/** One JSON line on stdout, written synchronously: a child past its cap reports and kills itself straight after. */
function emitLine(value: unknown): void {
  const buffer = Buffer.from(`${JSON.stringify(value)}\n`);
  for (let offset = 0; offset < buffer.length;) {
    try {
      offset += writeSync(1, buffer, offset);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EAGAIN") throw error;
    }
  }
}

/**
 * This process's CPU (user + system, every thread) since the last `restart`, checked every 20 ms on the JS thread,
 * which is idle while the addon evaluates: past `capMs`, `onPast` runs once. Each check also samples the resident set:
 * `peakRssBytes` is its peak since the watchdog started.
 */
class CpuWatchdog {
  #start = process.cpuUsage();
  #wallStart = performance.now();
  peakRssBytes = process.memoryUsage.rss();
  readonly #timer: NodeJS.Timeout;

  constructor(public capMs: number, onPast: (cpuMs: number, wallMs: number) => void) {
    this.#timer = setInterval(() => {
      this.peakRssBytes = Math.max(this.peakRssBytes, process.memoryUsage.rss());
      const cpuMs = this.cpuMs();
      if (cpuMs <= this.capMs) return;
      clearInterval(this.#timer);
      onPast(cpuMs, performance.now() - this.#wallStart);
    }, 20);
  }

  restart(capMs: number): void {
    this.capMs = capMs;
    this.#start = process.cpuUsage();
    this.#wallStart = performance.now();
  }

  /** This process's CPU since the last `restart`. */
  cpuMs(): number {
    const used = process.cpuUsage(this.#start);
    return (used.user + used.system) / 1000;
  }

  /** Stops the checks; the peak resident set, the current one included. */
  stop(): number {
    clearInterval(this.#timer);
    this.peakRssBytes = Math.max(this.peakRssBytes, process.memoryUsage.rss());
    return this.peakRssBytes;
  }
}

async function unitChild(options: Options): Promise<number> {
  const started = performance.now();
  const emit: (event: UnitEvent) => void = emitLine;
  const engine = loadEngine(options.addon);
  if (engine.addon.work !== "unbounded") throw new Error(`${engine.addon.path} is not an unbounded-work build`);
  // Read once per process: every unit this child runs takes its files from here.
  const files = options.scope === "full"
    ? collectRepository(engine, options.corpus).files
    : readFiles(engine, options.corpus, options.scope === "list" ? readFileSync(options.filesFrom!, "utf8").split("\n").filter(path => path.length > 0) : readSubset(options.subset).files.map(file => file.path));
  const byPath = new Map(files.map(file => [file.path, file]));
  emit({ kind: "ready", setupMs: performance.now() - started, files: files.length, addon: engine.addon, workerThreads: engine.workerThreads });
  // The last unit's compiled rule: the scheduler sends a unit's other engine next, so it compiles once (outside the timed apply).
  let compiled: { key: string; rules: PortedRule[] } | undefined;
  for await (const text of createInterface({ input: process.stdin })) {
    if (text.length === 0) continue;
    const request = requestSchema.parse(JSON.parse(text));
    const { unit } = request;
    let current: string | null = null;
    // The addon's work cannot be cancelled: past the cap the child reports and dies; the parent replaces it.
    const watchdog = new CpuWatchdog(request.capMs, (cpuMs, wallMs) => {
      emit({ kind: "timeout", id: request.id, cpuMs, wallMs, rssBytes: watchdog.peakRssBytes, path: current });
      process.kill(process.pid, "SIGKILL");
    });
    let opened: RuleWorkspace | undefined;
    try {
      const compileStarted = performance.now();
      if (compiled?.key !== JSON.stringify(unit)) compiled = { key: JSON.stringify(unit), rules: unit.linter === ALL ? loadRules(engine, options.policies) : [unitRule(engine, options.policies, unit)] };
      const { rules } = compiled;
      const compileMs = performance.now() - compileStarted;
      const linterOf = new Map(rules.map(rule => [documentSchema.parse(rule.document).id, rule.linter]));
      const name = (ruleId: string | undefined, clauseId: string) => `${linterOf.get(ruleId ?? "") ?? ruleId}/${clauseId}`;
      const matcher: Matcher = request.engine === "sql" ? { engine: "sql", ...(request.perFile ? { sqlBatchSize: 1 } : {}) } : { engine: "native" };
      const workspace = new RuleWorkspace(engine, rules, matcher);
      opened = workspace;
      const candidates = request.files === undefined ? files : request.files.map(path => byPath.get(path)!);
      const accepted = candidates.filter(file => workspace.accepts(file.path));
      emit({ kind: "start", id: request.id, files: accepted.map(file => file.path) });
      const load = loadavg()[0]!;
      // SQL: the program's lowering and emission, which the first engine report (or apply) runs. Timed apart, then
      // added to the unit's time, so the unit's total is what it was when the apply ran it.
      const programCpu = process.cpuUsage();
      const programStarted = performance.now();
      if (request.engine === "sql") workspace.engineReport();
      const programWallMs = request.engine === "sql" ? performance.now() - programStarted : 0;
      const programUsed = process.cpuUsage(programCpu);
      const programCpuMs = request.engine === "sql" ? (programUsed.user + programUsed.system) / 1000 : 0;
      let wallMs = programWallMs;
      let cpuMs = programCpuMs;
      const lines: string[] = [];
      if (request.perFile) {
        if (unit.linter === ALL && request.engine === "sql") {
          // Untimed: the whole program's compile and its session (batches of one), which every file would otherwise wait on first.
          await workspace.apply([{ path: "__rerail_sql_loop_program__.py", language: "python", source: PROGRAM_PROBE_SOURCE }]);
          await workspace.apply([], ["__rerail_sql_loop_program__.py"]);
        }
        for (const file of accepted) {
          current = file.path;
          watchdog.restart(request.fileCapMs ?? request.capMs);
          emit({ kind: "file-start", id: request.id, path: file.path });
          const timed = await workspace.apply([file]);
          await workspace.apply([], [file.path]);
          wallMs += timed.wallMs;
          cpuMs += timed.cpuMs;
          lines.push(...linesOf(timed.result));
          const report = request.engine === "sql" ? workspace.engineReport() : undefined;
          const onFile = report?.engine === "sql" ? (report.profile?.clauseFiles ?? []).filter(entry => entry.path === file.path) : [];
          emit({ kind: "file", id: request.id, path: file.path, wallMs: timed.wallMs, cpuMs: timed.cpuMs, clauses: onFile.map(entry => ({ clause: name(entry.ruleId, entry.clauseId), milliseconds: entry.milliseconds })) });
        }
        current = null;
      } else if (accepted.length > 0) {
        const timed = await workspace.apply(accepted);
        wallMs += timed.wallMs;
        cpuMs += timed.cpuMs;
        lines.push(...linesOf(timed.result));
      }
      const rssBytes = watchdog.stop();
      const report = workspace.engineReport();
      const sql = report.engine === "sql" ? report : undefined;
      const sqlMs = new Map<string, number>();
      for (const entry of sql?.profile?.clauses ?? []) sqlMs.set(name(entry.ruleId, entry.clauseId), (sqlMs.get(name(entry.ruleId, entry.clauseId)) ?? 0) + entry.milliseconds);
      const byClause = new Map(unit.clauses.map(clause => [clause, [] as string[]]));
      for (const line of lines) {
        const parsed = parseLine(line);
        byClause.get(name(parsed.rule, parsed.clause))?.push(line);
      }
      emit({
        kind: "unit", id: request.id, compileMs, wallMs, cpuMs, load, files: accepted.length,
        clauses: unit.clauses.map(clause => ({ clause, lines: byClause.get(clause)!.sort(compareStrings), sqlMs: sql === undefined ? null : sqlMs.get(clause) ?? 0 })),
        refused: sql?.refused.map(refusal => ({ clause: name(refusal.ruleId, refusal.clauseId), reason: refusal.reason })) ?? [],
        statements: sql?.statements.profile?.slice(0, UNIT_STATEMENTS) ?? [],
        clauseFiles: sql?.profile?.clauseFiles.slice(0, UNIT_CLAUSE_FILES).map(entry => ({ clause: name(entry.ruleId, entry.clauseId), path: entry.path, milliseconds: entry.milliseconds })) ?? [],
        rssBytes,
        phases: {
          ruleCompileMs: compileMs,
          programCpuMs,
          applyCpuMs: cpuMs - programCpuMs,
          sessionSetupMs: sql?.sessions?.setupMilliseconds ?? null,
          statementMs: sql?.statements.milliseconds ?? null,
          stageMs: sql?.statements.stageMilliseconds ?? null,
          selectMs: sql?.statements.selectMilliseconds ?? null,
          inputMs: sql?.statements.inputMilliseconds ?? null,
          factsMs: sql?.statements.factsMilliseconds ?? null,
          sessions: sql?.sessions?.count ?? null,
          statementsRun: sql?.statements.count ?? null,
          statementsSkipped: sql?.statements.skipped ?? null,
        },
        program: sql?.program ?? null,
      });
    } catch (error) {
      emit({ kind: "error", id: request.id, message: error instanceof Error ? error.message : String(error), cpuMs: watchdog.cpuMs(), rssBytes: watchdog.stop() });
    } finally {
      watchdog.stop();
      opened?.close();
    }
  }
  return 0;
}

/** A unit child seen from the parent: requests in, events out. */
class UnitChild {
  readonly #process: ChildProcess;
  readonly #events: UnitEvent[] = [];
  readonly #waiters: ((event: UnitEvent | undefined) => void)[] = [];
  #closed = false;

  constructor(options: Options, scope: Options["scope"], filesFrom?: string) {
    const args = [
      ...process.execArgv, script, "unit-child", "--addon", options.addon, "--corpus", options.corpus, "--policies", options.policies,
      "--subset", options.subset, "--scope", scope, ...(filesFrom === undefined ? [] : ["--files-from", filesFrom]),
    ];
    this.#process = spawn(process.execPath, args, { stdio: ["pipe", "pipe", "inherit"] });
    // A child killed at a cap leaves its stdin closed: writes after that are dropped, not thrown.
    this.#process.stdin!.on("error", () => {});
    createInterface({ input: this.#process.stdout! }).on("line", line => {
      const event = unitEventSchema.parse(JSON.parse(line));
      const waiter = this.#waiters.shift();
      if (waiter === undefined) this.#events.push(event);
      else waiter(event);
    });
    this.#process.on("close", () => {
      this.#closed = true;
      for (const waiter of this.#waiters.splice(0)) waiter(undefined);
    });
  }

  /** The next event, or undefined once the child exited, or "timeout" once `milliseconds` passed. */
  next(milliseconds: number): Promise<UnitEvent | undefined | "timeout"> {
    const queued = this.#events.shift();
    if (queued !== undefined) return Promise.resolve(queued);
    if (this.#closed) return Promise.resolve(undefined);
    const { promise, resolve: resolveNext } = Promise.withResolvers<UnitEvent | undefined | "timeout">();
    const waiter = (event: UnitEvent | undefined) => {
      clearTimeout(timer);
      resolveNext(event);
    };
    const timer = setTimeout(() => {
      this.#waiters.splice(this.#waiters.indexOf(waiter), 1);
      resolveNext("timeout");
    }, Math.max(0, milliseconds));
    this.#waiters.push(waiter);
    return promise;
  }

  send(request: UnitRequest): void {
    this.#process.stdin!.write(`${JSON.stringify(request)}\n`);
  }

  kill(): void {
    this.#process.kill("SIGKILL");
  }

  end(): void {
    this.#process.stdin!.end();
  }
}

/** Unit children over one scope, kept between units and batches: a child is replaced only when it dies. */
class UnitPool {
  readonly #idle: UnitChild[] = [];
  ready: ReadyEvent | undefined;

  constructor(readonly options: Options, readonly scope: Options["scope"], readonly filesFrom?: string) {}

  async acquire(): Promise<UnitChild> {
    const idle = this.#idle.pop();
    if (idle !== undefined) return idle;
    const spawned = new UnitChild(this.options, this.scope, this.filesFrom);
    const event = await spawned.next(10 * 60_000);
    if (event === undefined || event === "timeout" || event.kind !== "ready") throw new Error(`a unit child did not start (${event === undefined ? "exited" : event === "timeout" ? "timed out" : event.kind})`);
    this.ready ??= event;
    return spawned;
  }

  release(child: UnitChild): void {
    this.#idle.push(child);
  }

  close(): void {
    for (const child of this.#idle.splice(0)) child.end();
  }
}

/**
 * A unit's outcome on one engine: done, or killed past its CPU cap (TIMEOUT, with its CPU and wall time), or failed by
 * the engine, or its child died or stuck (ERROR). `rssBytes`: the child's peak resident set over the unit.
 */
type UnitOutcome = { engine: EngineName; unit: Unit } & (
  | ({ status: "ok" } & Omit<UnitDone, "kind" | "id">)
  | { status: "timeout" | "error"; wallMs: number; cpuMs: number | null; rssBytes: number | null; detail: string }
);

interface PairCost {
  clause: string;
  path: string;
  milliseconds: number;
  capped: boolean;
}

/**
 * A unit on one engine (`files`: over these files only, `costs`), or another job that takes a slot (the whole-program
 * setup); `estimateMs` orders them, `rssBytes` is the item's estimated peak resident set.
 */
type QueueItem = { estimateMs: number; rssBytes?: number } & (
  | { kind: "unit"; engine: EngineName; unit: Unit; files?: string[] }
  | { kind: "job"; run: () => Promise<void> }
);

interface UnitBatch {
  items: readonly QueueItem[];
  /** A clause alone: its CPU cap; per file: each file's. */
  capSeconds: number;
  /** A chunk of clauses: its CPU cap; per file: the setup's before the first file. */
  groupCapSeconds: number;
  jobs: number;
  /** The running items' estimated resident sets stay within this, unless one item alone passes it. */
  memoryBytes?: number;
  /** One file per apply: a capped file is recorded at the cap and the unit goes on without it. */
  perFile?: boolean;
  /** Called after each unit ends, however it ends. */
  onUnit?: (pairs: readonly PairCost[], outcomes: readonly UnitOutcome[]) => void;
  label: string;
}

/**
 * Runs `batch`'s items longest estimate first (LPT) in `jobs` slots, each unit in a long-lived child of `pool`. A slot
 * takes the first item whose estimated resident set fits beside the running ones (`memoryBytes`), and waits while none
 * does; after a unit it takes the same unit on the other engine first, so the child compiles the unit's rule once and
 * both engines run it under the same load. A unit past its CPU cap kills its child: a TIMEOUT row with its CPU and
 * wall time, never split or retried (per file: the file is recorded at the cap and the unit goes on without it). A
 * unit the engine fails is an ERROR row; a child whose wall time passes `WALL_GUARD` times its cap is killed as stuck,
 * and one that exits is lost: ERROR rows too.
 */
async function runUnits(pool: UnitPool, batch: UnitBatch): Promise<{ outcomes: UnitOutcome[]; pairs: PairCost[]; wallMs: number; peakRunning: number }> {
  const started = performance.now();
  const queue = [...batch.items].sort((a, b) => b.estimateMs - a.estimateMs);
  const outcomes: UnitOutcome[] = [];
  const pairs: PairCost[] = [];
  let nextId = 0;
  let running = 0;
  let peakRunning = 0;
  let reservedBytes = 0;
  let freed = Promise.withResolvers<void>();
  const take = async (after: Unit | undefined): Promise<QueueItem | undefined> => {
    for (;;) {
      if (queue.length === 0) return undefined;
      const fits = (item: QueueItem) => reservedBytes + (item.rssBytes ?? 0) <= (batch.memoryBytes ?? Infinity);
      const twin = after === undefined ? -1 : queue.findIndex(item => item.kind === "unit" && unitKey(item.unit) === unitKey(after) && fits(item));
      const index = twin >= 0 ? twin : queue.findIndex(fits);
      if (index >= 0 || running === 0) {
        const [item] = queue.splice(Math.max(0, index), 1);
        running += 1;
        peakRunning = Math.max(peakRunning, running);
        reservedBytes += item!.rssBytes ?? 0;
        return item;
      }
      await freed.promise;
    }
  };
  const done = (item: QueueItem) => {
    running -= 1;
    reservedBytes -= item.rssBytes ?? 0;
    freed.resolve();
    freed = Promise.withResolvers<void>();
  };
  const worker = async () => {
    let child: UnitChild | undefined;
    let last: Unit | undefined;
    for (let item = await take(last); item !== undefined; item = await take(last)) {
      if (item.kind === "job") {
        await item.run();
        done(item);
        last = undefined;
        continue;
      }
      const { unit, engine } = item;
      last = unit;
      child ??= await pool.acquire();
      const id = nextId++;
      const unitStarted = performance.now();
      const fileCapMs = batch.capSeconds * 1000;
      const capMs = batch.perFile || unit.clauses.length > 1 ? batch.groupCapSeconds * 1000 : fileCapMs;
      child.send({ id, engine, unit, ...(item.files === undefined ? {} : { files: item.files }), perFile: batch.perFile ?? false, capMs, ...(batch.perFile ? { fileCapMs } : {}) });
      let guard = unitStarted + capMs * WALL_GUARD + 60_000;
      let accepted: string[] = [];
      const finished = new Set<string>();
      // Per file, a unit's whole-file times go under its clause (alone) or "*" (several clauses).
      const clause = unit.clauses.length === 1 ? unit.clauses[0]! : ALL;
      for (;;) {
        const event = await child.next(guard - performance.now());
        if (event === "timeout" || event === undefined) {
          child.kill();
          child = undefined;
          const elapsed = performance.now() - unitStarted;
          const detail = event === undefined ? "its child exited" : `its child was stuck: ${seconds(elapsed)} wall without reaching its ${capMs / 1000} s CPU cap`;
          outcomes.push({ status: "error", engine, unit, wallMs: elapsed, cpuMs: null, rssBytes: null, detail });
          process.stderr.write(`  ${batch.label}: ${engine} ${unitKey(unit)} ERROR: ${detail}\n`);
          batch.onUnit?.(pairs, outcomes);
          break;
        }
        if (event.kind === "timeout" && event.id === id) {
          // The child kills itself after the event; this makes sure.
          child.kill();
          child = undefined;
          if (batch.perFile && event.path !== null) {
            const { path } = event;
            pairs.push({ clause, path, milliseconds: event.wallMs, capped: true });
            process.stderr.write(`  ${batch.label}: ${clause} @ ${path} passed the ${batch.capSeconds} s CPU cap\n`);
            queue.unshift({ ...item, files: accepted.filter(file => !finished.has(file) && file !== path) });
          } else {
            outcomes.push({ status: "timeout", engine, unit, wallMs: event.wallMs, cpuMs: event.cpuMs, rssBytes: event.rssBytes, detail: `killed past the ${capMs / 1000} s CPU cap` });
            process.stderr.write(`  ${batch.label}: ${engine} ${unitKey(unit)} TIMEOUT at ${seconds(event.cpuMs)} CPU, ${seconds(event.wallMs)} wall\n`);
          }
          batch.onUnit?.(pairs, outcomes);
          break;
        }
        if (event.kind === "error" && event.id === id) {
          // The child stays alive: the engine failed this unit only.
          outcomes.push({ status: "error", engine, unit, wallMs: performance.now() - unitStarted, cpuMs: event.cpuMs, rssBytes: event.rssBytes, detail: event.message });
          process.stderr.write(`  ${batch.label}: ${engine} ${unitKey(unit)} ERROR: ${event.message.slice(0, 200)}\n`);
          batch.onUnit?.(pairs, outcomes);
          break;
        }
        if (event.kind === "start" && event.id === id) accepted = event.files;
        else if (event.kind === "file-start" && event.id === id) guard = performance.now() + fileCapMs * WALL_GUARD + 60_000;
        else if (event.kind === "file" && event.id === id) {
          finished.add(event.path);
          pairs.push({ clause, path: event.path, milliseconds: event.wallMs, capped: false });
          if (unit.clauses.length !== 1) for (const entry of event.clauses) pairs.push({ clause: entry.clause, path: event.path, milliseconds: entry.milliseconds, capped: false });
        } else if (event.kind === "unit" && event.id === id) {
          const { kind: _kind, id: _id, ...rest } = event;
          outcomes.push({ ...rest, status: "ok", engine, unit });
          batch.onUnit?.(pairs, outcomes);
          break;
        }
      }
      done(item);
    }
    if (child !== undefined) pool.release(child);
  };
  await Promise.all(Array.from({ length: Math.min(batch.jobs, Math.max(1, queue.length)) }, worker));
  return { outcomes, pairs, wallMs: performance.now() - started, peakRunning };
}

// ---------------------------------------------------------------------------------------------------------------------
// The whole-program SQL setup: every clause in one program, compiled, and every worker's session

const programEventSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("program"), compileMs: z.number(), setupMs: z.number(), cpuMs: z.number(), rssBytes: z.number(), clauses: z.number().int() }),
  /** Past `--program-cap` CPU seconds: the child dies right after this event. */
  z.object({ kind: z.literal("timeout"), cpuMs: z.number(), wallMs: z.number(), rssBytes: z.number() }),
]);
type ProgramEvent = z.infer<typeof programEventSchema>;

/**
 * A file with the common Python constructs: a session opens only for a batch with candidate roots for some clause
 * (`workspace/evaluate.rs`, `sql_batch`), and every session creates the whole program's tables.
 */
const PROGRAM_PROBE_SOURCE = [
  "import os", "from typing import Any", "", "class Store(Base):", "    name: str = 'x'", "",
  "    async def get(self, key: str) -> Any:", "        if hasattr(self, key) and key in self.items:", "            return await self.session.execute(key)",
  "        for item in self.items:", "            with open(item) as handle:", "                yield handle.read()", "        try:", "            value = getattr(self, key, None)",
  "        except Exception as error:", "            raise RuntimeError(f'{key}') from error", "        return lambda: [value, {key: value}, (1, 2)]", "",
  "def resolve(ref, *args, **kwargs):", "    return Store().get(ref.id).result() if ref is not None else ref == 1", "",
].join("\n");

async function programChild(options: Options): Promise<number> {
  const watchdog = new CpuWatchdog(options.programCapSeconds * 1000, (cpuMs, wallMs) => {
    emitLine({ kind: "timeout", cpuMs, wallMs, rssBytes: watchdog.peakRssBytes } satisfies ProgramEvent);
    process.kill(process.pid, "SIGKILL");
  });
  const cpu = process.cpuUsage();
  const engine = loadEngine(options.addon);
  const compileStarted = performance.now();
  const rules = loadRules(engine, options.policies);
  const compileMs = performance.now() - compileStarted;
  const workspace = new RuleWorkspace(engine, rules, { engine: "sql" });
  try {
    const started = performance.now();
    // `serverplan_quality` scopes `**/*.py`: one batch per worker, so the program compiles and every session opens.
    const dummies = Array.from({ length: engine.workerThreads }, (_, index): SourceFile => ({ path: `__rerail_sql_loop_program_${index}__.py`, language: "python", source: PROGRAM_PROBE_SOURCE }));
    await workspace.apply(dummies);
    const setupMs = performance.now() - started;
    const rssBytes = watchdog.stop();
    const used = process.cpuUsage(cpu);
    emitLine({ kind: "program", compileMs, setupMs, cpuMs: (used.user + used.system) / 1000, rssBytes, clauses: reportingClauses(rules).length } satisfies ProgramEvent);
  } finally {
    watchdog.stop();
    workspace.close();
  }
  return 0;
}

/** The whole-program setup; `rssBytes`: its child's peak resident set. */
type ProgramOutcome =
  | { status: "ok"; compileMs: number; setupMs: number; cpuMs: number; rssBytes: number; clauses: number }
  | { status: "timeout"; cpuMs: number; wallMs: number; rssBytes: number }
  | { status: "error"; wallMs: number };

/** The whole-program setup in its own child, capped at `--program-cap` CPU seconds (and `WALL_GUARD` times that in wall time). */
async function runProgram(options: Options): Promise<ProgramOutcome> {
  const started = performance.now();
  const child = spawn(process.execPath, [...process.execArgv, script, "program-child", "--addon", options.addon, "--policies", options.policies, "--program-cap", String(options.programCapSeconds)], { stdio: ["ignore", "pipe", "inherit"] });
  let found: ProgramEvent | undefined;
  createInterface({ input: child.stdout }).on("line", line => {
    found = programEventSchema.parse(JSON.parse(line));
  });
  const guard = setTimeout(() => child.kill("SIGKILL"), options.programCapSeconds * 1000 * WALL_GUARD + 60_000);
  const exited = Promise.withResolvers<void>();
  child.on("close", () => exited.resolve());
  await exited.promise;
  clearTimeout(guard);
  if (found?.kind === "program") return { status: "ok", compileMs: found.compileMs, setupMs: found.setupMs, cpuMs: found.cpuMs, rssBytes: found.rssBytes, clauses: found.clauses };
  if (found?.kind === "timeout") return { status: "timeout", cpuMs: found.cpuMs, wallMs: found.wallMs, rssBytes: found.rssBytes };
  return { status: "error", wallMs: performance.now() - started };
}

// ---------------------------------------------------------------------------------------------------------------------
// The native cache: the oracle (native's lines over the subset) and native's whole-program times

/** sha256 over the native sources outside `sql/` (working tree, untracked included) and the cargo manifests. */
function nativeSourceHash(): string {
  const listed = git(rerailRoot, "ls-files", "-z", "-co", "--exclude-standard", "--", "native", "Cargo.toml", "Cargo.lock")
    .split("\0")
    .filter(path => path.length > 0 && !path.startsWith("native/src/code_rule_dsl/sql/") && existsSync(resolve(rerailRoot, path)))
    .sort(compareStrings);
  const hash = createHash("sha256");
  for (const path of listed) hash.update(path).update("\0").update(readFileSync(resolve(rerailRoot, path))).update("\0");
  return `sha256:${hash.digest("hex")}`;
}

function policiesHash(policies: string): Record<string, string> {
  const hashes: Record<string, string> = {};
  for (const entry of readdirSync(policies, { withFileTypes: true }).filter(entry => entry.isDirectory()).sort((a, b) => compareStrings(a.name, b.name))) {
    const path = resolve(policies, entry.name, "policy.json");
    if (existsSync(path)) hashes[entry.name] = sha256(readFileSync(path));
  }
  return hashes;
}

const cacheSchema = z.object({
  format: z.literal("rerail.sql-loop-native-cache/v3"),
  key: z.record(z.string(), z.string()),
  measured: z.string(),
  /** Per clause: native's lines over the subset alone, every clause in one workspace. */
  lines: z.record(z.string(), z.array(z.string())),
  /** Every clause in one workspace (production's shape): native's whole-tree and subset apply. */
  whole: z.object({ full: z.object({ wallMs: z.number(), cpuMs: z.number(), files: z.number().int() }), subset: z.object({ wallMs: z.number(), cpuMs: z.number() }) }),
});
type NativeCache = z.infer<typeof cacheSchema>;

function nativeCacheKey(options: Options, subset: Subset) {
  return {
    format: "rerail.sql-loop-native-cache/v3",
    nativeSources: nativeSourceHash(),
    addonBuild: options.addon.replace(rerailRoot, "."),
    subset: subsetSha(subset),
    policies: sha256(JSON.stringify(policiesHash(options.policies))),
    corpus: git(options.corpus, "rev-parse", "HEAD").trim(),
  };
}

function cachePathOf(key: Record<string, string>): string {
  return resolve(stateRoot, "loop-cache", `${sha256(JSON.stringify(key)).slice("sha256:".length, "sha256:".length + 24)}.json`);
}

/** The native cache for `key`, measured on a miss: native over the subset (the oracle) and the whole tree, every clause in one workspace each. */
async function nativeCache(options: Options, subset: Subset, key: Record<string, string>): Promise<{ cache: NativeCache; measured: boolean }> {
  const path = cachePathOf(key);
  if (existsSync(path)) return { cache: cacheSchema.parse(JSON.parse(readFileSync(path, "utf8"))), measured: false };
  process.stderr.write("native cache: miss; native over the subset (the oracle) and the whole tree\n");
  const engine = loadEngine(options.addon);
  const rules = loadRules(engine, options.policies);
  const timedWhole = async (files: readonly SourceFile[]) => {
    const workspace = new RuleWorkspace(engine, rules, { engine: "native" });
    try {
      return await workspace.apply(files.filter(file => workspace.accepts(file.path)));
    } finally {
      workspace.close();
    }
  };
  const repository = collectRepository(engine, options.corpus);
  const fullApply = await timedWhole(repository.files);
  const subsetApply = await timedWhole(readFiles(engine, options.corpus, subset.files.map(file => file.path)));
  const linterOf = new Map(rules.map(rule => [documentSchema.parse(rule.document).id, rule.linter]));
  const lines: NativeCache["lines"] = Object.fromEntries(reportingClauses(rules).map(clause => [clause.name, [] as string[]]));
  for (const line of linesOf(subsetApply.result)) {
    const parsed = parseLine(line);
    lines[`${linterOf.get(parsed.rule) ?? parsed.rule}/${parsed.clause}`]?.push(line);
  }
  const cache: NativeCache = {
    format: "rerail.sql-loop-native-cache/v3", key, measured: new Date().toISOString(), lines,
    whole: { full: { wallMs: fullApply.wallMs, cpuMs: fullApply.cpuMs, files: repository.files.length }, subset: { wallMs: subsetApply.wallMs, cpuMs: subsetApply.cpuMs } },
  };
  mkdirSync(resolve(stateRoot, "loop-cache"), { recursive: true });
  writeFileSync(path, `${JSON.stringify(cache)}\n`);
  return { cache, measured: true };
}

// ---------------------------------------------------------------------------------------------------------------------
// run

const median = (values: readonly number[]) => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length === 0 ? Number.NaN : sorted.length % 2 === 1 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
};
const spread = (values: readonly number[]) => (values.length < 2 ? 0 : (Math.max(...values) - Math.min(...values)) / Math.min(...values));
const seconds = (ms: number) => `${(ms / 1000).toFixed(2)} s`;
const percent = (value: number) => `${value >= 0 ? "+" : ""}${(value * 100).toFixed(1)}%`;
const sum = (values: Iterable<number>) => {
  let total = 0;
  for (const value of values) total += value;
  return total;
};

const unitRecordSchema = z.object({
  unit: unitSchema,
  status: z.enum(["ok", "timeout", "error"]),
  wallMs: z.number(),
  cpuMs: z.number().nullable(),
  /** The child's peak resident set over the unit (v4 on). */
  rssBytes: z.number().nullable().optional(),
  /** Where the unit's CPU went (finished units, from v4 runs with phases). */
  phases: phasesSchema.nullable().optional(),
  program: z.record(z.string(), z.number()).nullable().optional(),
  /** Per clause: its SQL statement time (SQL only). */
  sqlMs: z.record(z.string(), z.number()).nullable(),
  detail: z.string().nullable(),
});
type UnitRecord = z.infer<typeof unitRecordSchema>;
/** v3 rounds ran SQL then native in phases (and split chunks); v4 rounds run both engines' units in one LPT queue. */
const roundRecordSchema = z.looseObject({ sql: z.array(unitRecordSchema), native: z.array(unitRecordSchema) });
const runRecordSchema = z.looseObject({
  format: z.enum(["rerail.sql-loop-run/v3", "rerail.sql-loop-run/v4"]),
  started: z.string(),
  addon: z.looseObject({ sha256: z.string() }),
  subsetSha256: z.string(),
  clauseSetSha256: z.string(),
  rounds: z.array(roundRecordSchema),
  /** Per round, over the units both engines finished. */
  totals: z.object({ sqlCpuMs: z.array(z.number()), sqlWallMs: z.array(z.number()), nativeCpuMs: z.array(z.number()), nativeWallMs: z.array(z.number()) }),
  ratio: z.object({ wall: z.number(), cpu: z.number() }),
  program: z.looseObject({ rssBytes: z.number().optional() }).nullable().optional(),
});
type RunRecord = z.infer<typeof runRecordSchema>;

function runRecords(): { path: string; record: RunRecord }[] {
  const directory = resolve(stateRoot, "loop-runs");
  if (!existsSync(directory)) return [];
  return readdirSync(directory).filter(name => name.endsWith(".json")).sort(compareStrings).reverse().flatMap(name => {
    const path = resolve(directory, name);
    const parsed = runRecordSchema.safeParse(JSON.parse(readFileSync(path, "utf8")));
    return parsed.success ? [{ path, record: parsed.data }] : [];
  });
}

const unitRecord = (outcome: UnitOutcome): UnitRecord => outcome.status === "ok"
  ? { unit: outcome.unit, status: "ok", wallMs: outcome.wallMs, cpuMs: outcome.cpuMs, rssBytes: outcome.rssBytes, phases: outcome.phases, program: outcome.program, detail: null, sqlMs: outcome.clauses.some(clause => clause.sqlMs !== null) ? Object.fromEntries(outcome.clauses.map(clause => [clause.clause, clause.sqlMs ?? 0])) : null }
  : { unit: outcome.unit, status: outcome.status, wallMs: outcome.wallMs, cpuMs: outcome.cpuMs, rssBytes: outcome.rssBytes, sqlMs: null, detail: outcome.detail };

/** What units cost, from previous runs on this subset: the estimates the chunking, the LPT order and the memory budget use. */
interface CostModel {
  /** Per clause and engine: its CPU, a unit's CPU above the overhead shared by statement time (native: evenly); Infinity: its unit timed out or failed. */
  clauses: Record<EngineName, Map<string, number>>;
  /** Per engine: a unit's CPU apart from its clauses (rule compile, parsing, SQL program and session setup). */
  overheadMs: Record<EngineName, number>;
  /** Per engine: the peak resident set of each unit (by key), and of each clause (the largest unit it was in). */
  unitRssBytes: Record<EngineName, Map<string, number>>;
  clauseRssBytes: Record<EngineName, Map<string, number>>;
  programRssBytes: number;
  runs: number;
}

/** Newest run first: a clause's or unit's cost is its latest run's (the costliest round there). */
function costModel(records: readonly RunRecord[]): CostModel {
  const clauses: CostModel["clauses"] = { sql: new Map(), native: new Map() };
  const unitRssBytes: CostModel["unitRssBytes"] = { sql: new Map(), native: new Map() };
  const clauseRssBytes: CostModel["clauseRssBytes"] = { sql: new Map(), native: new Map() };
  const overheadMs: Partial<Record<EngineName, number>> = {};
  for (const record of records) {
    const units = { sql: record.rounds.flatMap(round => round.sql), native: record.rounds.flatMap(round => round.native) };
    const finished = (engine: EngineName) => units[engine].filter(unit => unit.status === "ok" && unit.cpuMs !== null);
    // The overhead: SQL, a unit's CPU outside its statements; native, a clause alone's CPU (its own work is small).
    const recordOverheads = {
      sql: median(finished("sql").map(unit => Math.max(0, unit.cpuMs! - sum(Object.values(unit.sqlMs ?? {}))))),
      native: median(finished("native").filter(unit => unit.unit.clauses.length === 1).map(unit => unit.cpuMs!)),
    };
    for (const engine of ["sql", "native"] as const) {
      if (!Number.isNaN(recordOverheads[engine])) overheadMs[engine] ??= recordOverheads[engine];
      const overhead = Number.isNaN(recordOverheads[engine]) ? 0 : recordOverheads[engine];
      const local = new Map<string, number>();
      const localUnitRss = new Map<string, number>();
      const localClauseRss = new Map<string, number>();
      for (const unit of units[engine]) {
        const above = unit.status === "ok" && unit.cpuMs !== null ? Math.max(0, unit.cpuMs - overhead) : Infinity;
        const statements = sum(unit.unit.clauses.map(clause => unit.sqlMs?.[clause] ?? 0));
        for (const clause of unit.unit.clauses) {
          const share = above === Infinity ? Infinity : statements > 0 ? above * (unit.sqlMs?.[clause] ?? 0) / statements : above / unit.unit.clauses.length;
          local.set(clause, Math.max(local.get(clause) ?? 0, share));
          if (typeof unit.rssBytes === "number") localClauseRss.set(clause, Math.max(localClauseRss.get(clause) ?? 0, unit.rssBytes));
        }
        if (typeof unit.rssBytes === "number") localUnitRss.set(unitKey(unit.unit), Math.max(localUnitRss.get(unitKey(unit.unit)) ?? 0, unit.rssBytes));
      }
      for (const [clause, milliseconds] of local) if (!clauses[engine].has(clause)) clauses[engine].set(clause, milliseconds);
      for (const [key, bytes] of localUnitRss) if (!unitRssBytes[engine].has(key)) unitRssBytes[engine].set(key, bytes);
      for (const [clause, bytes] of localClauseRss) if (!clauseRssBytes[engine].has(clause)) clauseRssBytes[engine].set(clause, bytes);
    }
  }
  return {
    clauses, unitRssBytes, clauseRssBytes,
    overheadMs: { sql: overheadMs.sql ?? 600, native: overheadMs.native ?? 600 },
    programRssBytes: records.find(record => typeof record.program?.rssBytes === "number")?.program?.rssBytes ?? UNKNOWN_PROGRAM_RSS_BYTES,
    runs: records.length,
  };
}

/** A unit with its estimated CPU (a clause that timed out counts at its cap) and peak resident set per engine. */
interface PlannedUnit {
  unit: Unit;
  estimateMs: Record<EngineName, number>;
  rssBytes: Record<EngineName, number>;
}

/**
 * The units, decided up front. The SQL/native CPU ratio depends on the units (each parses the files once on each
 * engine, and that dominates native's time), so the units stay those of the previous run: the ratio then moves only
 * with the engines. A chunk that did not finish there (TIMEOUT or ERROR) splits into its clauses, each alone. A policy
 * is packed afresh only when its clauses changed, or on `--replan`: a clause whose SQL cost reached `ISOLATE_MS` (or
 * that did not finish) alone; the policy's other clauses, in order, in chunks of up to `--chunk` seconds of estimated
 * SQL CPU, each one workspace (one parse).
 */
function planUnits(clauses: readonly Clause[], options: Options, model: CostModel, previous: readonly UnitRecord[]): PlannedUnit[] {
  const sqlCost = (clause: string) => model.clauses.sql.get(clause) ?? UNKNOWN_CLAUSE_MS;
  const estimate = (unit: Unit): PlannedUnit => {
    const capMs = (unit.clauses.length > 1 ? options.groupCapSeconds : options.capSeconds) * 1000;
    // A unit no run measured: the largest unit any of its clauses ran in.
    const rss = (engine: EngineName) => model.unitRssBytes[engine].get(unitKey(unit)) ?? (Math.max(0, ...unit.clauses.map(clause => model.clauseRssBytes[engine].get(clause) ?? 0)) || UNKNOWN_UNIT_RSS_BYTES);
    return {
      unit,
      estimateMs: {
        sql: Math.min(capMs, model.overheadMs.sql + sum(unit.clauses.map(sqlCost))),
        native: Math.min(capMs, model.overheadMs.native + sum(unit.clauses.map(clause => Math.min(capMs, model.clauses.native.get(clause) ?? 0)))),
      },
      rssBytes: { sql: rss("sql"), native: rss("native") },
    };
  };
  const units: PlannedUnit[] = [];
  for (const linter of new Set(clauses.map(clause => clause.linter))) {
    const own = clauses.filter(clause => clause.linter === linter).map(clause => clause.name);
    const kept = previous.filter(record => record.unit.linter === linter);
    const keptClauses = new Set(kept.flatMap(record => record.unit.clauses));
    if (!options.replan && keptClauses.size === own.length && own.every(clause => keptClauses.has(clause))) {
      for (const record of kept) {
        if (record.status === "ok" || record.unit.clauses.length === 1) units.push(estimate(record.unit));
        else units.push(...record.unit.clauses.map(clause => estimate({ linter, clauses: [clause] })));
      }
      continue;
    }
    let chunk: string[] = [];
    let chunkMs = 0;
    for (const clause of own) {
      const cost = sqlCost(clause);
      if (cost >= ISOLATE_MS) {
        units.push(estimate({ linter, clauses: [clause] }));
        continue;
      }
      if (chunk.length > 0 && chunkMs + cost > options.chunkSeconds * 1000) {
        units.push(estimate({ linter, clauses: chunk }));
        chunk = [];
        chunkMs = 0;
      }
      chunk.push(clause);
      chunkMs += cost;
    }
    if (chunk.length > 0) units.push(estimate({ linter, clauses: chunk }));
  }
  return units;
}

async function run(options: Options): Promise<number> {
  const startedAt = new Date();
  const started = performance.now();
  const backgroundLoad = loadavg();
  const subset = readSubset(options.subset);
  const engine = loadEngine(options.addon);
  const clauses = reportingClauses(loadRules(engine, options.policies), options.clauses);
  if (clauses.length === 0) throw new UsageError(`no clause matches --clauses ${options.clauses.join(",")}`);
  const names = clauses.map(clause => clause.name);
  const clauseSetSha = sha256(JSON.stringify(names));
  const stamp = startedAt.toISOString().replace(/[:.]/g, "-");
  const history = runRecords().filter(({ record }) => record.subsetSha256 === subsetSha(subset));
  const previous = history.find(({ record }) => record.clauseSetSha256 === clauseSetSha);
  const model = costModel(history.slice(0, COST_HISTORY).map(({ record }) => record));
  // v3 runs split chunks within the run: their units are no plan to keep.
  const planned = planUnits(clauses, options, model, previous?.record.format === "rerail.sql-loop-run/v4" ? previous.record.rounds[0]!.sql : []);
  const estimated = (engineName: EngineName) => seconds(sum(planned.map(unit => unit.estimateMs[engineName])));
  const gib = (bytes: number) => `${(bytes / 2 ** 30).toFixed(1)} GiB`;
  const largest = Math.max(...planned.map(unit => unit.rssBytes.sql));
  console.log(`sql-loop: ${names.length} clauses in ${planned.length} units (${planned.filter(unit => unit.unit.clauses.length === 1).length} alone; chunks ≤ ${options.chunkSeconds} s SQL CPU by the costs of ${model.runs} previous runs) over ${subset.files.length} subset files; estimated CPU SQL ${estimated("sql")}, native ${estimated("native")}; ${options.rounds} round(s), ${options.jobs} jobs within ${options.memoryGib} GiB (largest SQL unit ${gib(largest)}, program ${gib(model.programRssBytes)}), longest first; CPU caps ${options.capSeconds} s alone, ${options.groupCapSeconds} s per chunk; background load ${backgroundLoad.map(value => value.toFixed(1)).join(" ")}`);

  const key = nativeCacheKey(options, subset);
  const { cache, measured } = await nativeCache(options, subset, key);
  if (measured) console.log(`  native cache: measured (whole tree ${seconds(cache.whole.full.wallMs)} wall)`);
  const pool = new UnitPool(options, "subset");
  const rounds: { sql: UnitOutcome[]; native: UnitOutcome[]; wallMs: number; peakRunning: number }[] = [];
  const timedOut = new Set<string>();
  let program: ProgramOutcome | undefined;
  for (let round = 1; round <= options.rounds; round += 1) {
    // Both engines' units in one queue, longest first; later rounds leave out the units SQL did not finish.
    const live = planned.filter(unit => !timedOut.has(unitKey(unit.unit)));
    const items: QueueItem[] = (["sql", "native"] as const).flatMap(engineName => live.map((unit): QueueItem => ({ kind: "unit", engine: engineName, unit: unit.unit, estimateMs: unit.estimateMs[engineName], rssBytes: unit.rssBytes[engineName] })));
    if (round === 1 && options.programCapSeconds > 0) {
      items.push({ kind: "job", estimateMs: options.programCapSeconds * 1000, rssBytes: model.programRssBytes, run: async () => {
        program = await runProgram(options);
      } });
    }
    const result = await runUnits(pool, { items, capSeconds: options.capSeconds, groupCapSeconds: options.groupCapSeconds, jobs: options.jobs, memoryBytes: options.memoryGib * 2 ** 30, label: `round ${round}` });
    const sql = result.outcomes.filter(outcome => outcome.engine === "sql");
    for (const outcome of sql) if (outcome.status !== "ok") timedOut.add(unitKey(outcome.unit));
    rounds.push({ sql, native: result.outcomes.filter(outcome => outcome.engine === "native"), wallMs: result.wallMs, peakRunning: result.peakRunning });
    const peakRss = Math.max(0, ...result.outcomes.map(outcome => outcome.rssBytes ?? 0));
    process.stderr.write(`  round ${round}: ${seconds(result.wallMs)} wall, up to ${result.peakRunning} of ${options.jobs} jobs at once; ${sql.filter(outcome => outcome.status !== "ok").length} SQL units past their cap; largest unit ${gib(peakRss)} resident\n`);
  }
  pool.close();

  const problems: string[] = [];
  // Parity: each clause's lines in the last round SQL finished it.
  console.log("");
  const sqlLines = new Map<string, string[]>();
  const refused = new Map<string, string>();
  for (const round of rounds) {
    for (const outcome of round.sql) {
      if (outcome.status !== "ok") continue;
      for (const clause of outcome.clauses) {
        const before = sqlLines.get(clause.clause);
        if (before !== undefined && before.join("\n") !== clause.lines.join("\n")) problems.push(`SQL rounds disagree on ${clause.clause}`);
        sqlLines.set(clause.clause, clause.lines);
      }
      for (const refusal of outcome.refused) refused.set(refusal.clause, refusal.reason);
    }
    for (const outcome of round.native) {
      if (outcome.status !== "ok") problems.push(`native ${outcome.status} on ${unitKey(outcome.unit)}`);
      else for (const clause of outcome.clauses) if (clause.lines.join("\n") !== (cache.lines[clause.clause] ?? []).join("\n")) problems.push(`native's lines for ${clause.clause} in its unit differ from native's over every clause together`);
    }
  }
  const parities = [...sqlLines].map(([clause, lines]) => compareLines(clause, cache.lines[clause] ?? [], lines));
  const differing = parities.filter(parity => parity.files.length > 0);
  // Not compared: the clauses of the SQL units that passed their cap (TIMEOUT) or that the engine failed (ERROR).
  const notCompared = names.filter(clause => !sqlLines.has(clause));
  const errored = new Set(rounds.flatMap(round => round.sql.flatMap(outcome => (outcome.status === "error" ? outcome.unit.clauses : []))));
  const timeouts = notCompared.filter(clause => !errored.has(clause));
  const errors = notCompared.filter(clause => errored.has(clause));
  const notComparedText = [timeouts.length > 0 ? `${timeouts.length} TIMEOUT` : "", errors.length > 0 ? `${errors.length} ERROR` : ""].filter(text => text.length > 0).join(", ");
  console.log(`PARITY ${differing.length === 0 ? "OK" : "DIFFERS"}: ${parities.length} clauses compared (${sum(parities.map(parity => parity.compared))} native lines), ${differing.length} differ; ${notCompared.length} not compared${notCompared.length > 0 ? ` (${notComparedText})` : ""}; ${sum(parities.map(parity => parity.budgetExcluded.length))} (clause, file)s excluded for native budget failures; SQL refused ${refused.size} clauses`);
  for (const parity of differing.slice(0, options.show)) {
    console.log(`  ${parity.clause}: ${parity.files.length} files differ`);
    for (const file of parity.files.slice(0, 3)) {
      console.log(`    @ ${file.path || "(no file)"}: ${file.onlyNative.length} only native, ${file.onlySql.length} only SQL`);
      for (const line of file.onlyNative.slice(0, 2)) console.log(`      - native: ${line.slice(0, 240)}`);
      for (const line of file.onlySql.slice(0, 2)) console.log(`      + sql:    ${line.slice(0, 240)}`);
    }
  }
  for (const [clause, reason] of [...refused].slice(0, options.show)) console.log(`  refused ${clause}: ${reason}`);

  // Speed: per round, over the units both engines finished.
  const totals = rounds.map(round => {
    const nativeByKey = new Map(round.native.flatMap(outcome => (outcome.status === "ok" ? [[unitKey(outcome.unit), outcome] as const] : [])));
    const both = round.sql.flatMap(outcome => (outcome.status === "ok" && nativeByKey.has(unitKey(outcome.unit)) ? [{ sql: outcome, native: nativeByKey.get(unitKey(outcome.unit))! }] : []));
    return {
      sqlCpuMs: sum(both.map(pair => pair.sql.cpuMs)), sqlWallMs: sum(both.map(pair => pair.sql.wallMs)),
      nativeCpuMs: sum(both.map(pair => pair.native.cpuMs)), nativeWallMs: sum(both.map(pair => pair.native.wallMs)),
      timeoutCpuMs: sum(round.sql.flatMap(outcome => (outcome.status === "ok" ? [] : [outcome.cpuMs ?? outcome.wallMs]))),
      units: both.length,
    };
  });
  const sqlCpu = totals.map(total => total.sqlCpuMs);
  const sqlWall = totals.map(total => total.sqlWallMs);
  const nativeCpu = totals.map(total => total.nativeCpuMs);
  const nativeWall = totals.map(total => total.nativeWallMs);
  const ratio = { wall: median(sqlWall) / median(nativeWall), cpu: median(sqlCpu) / median(nativeCpu) };
  const untrusted: string[] = [];
  if (backgroundLoad[0]! > LOAD_LIMIT) untrusted.push(`background load ${backgroundLoad[0]!.toFixed(1)} > ${LOAD_LIMIT}`);
  // Every round runs the same units (later rounds without SQL's TIMEOUTs, which are outside the sums anyway).
  if (spread(sqlCpu) > SPREAD_LIMIT) untrusted.push(`SQL rounds spread ${percent(spread(sqlCpu))} in CPU`);
  if (spread(nativeCpu) > SPREAD_LIMIT) untrusted.push(`native rounds spread ${percent(spread(nativeCpu))} in CPU`);
  if (totals.length < 2) untrusted.push(`${totals.length} round(s): spread unknown`);
  if (notCompared.length > 0) untrusted.push(`${notComparedText} clauses outside the ratio`);
  const unitLoads = rounds.flatMap(round => round.sql.flatMap(outcome => (outcome.status === "ok" ? [outcome.load] : [])));
  console.log("");
  console.log(`SPEED${untrusted.length > 0 ? ` (UNTRUSTED: ${untrusted.join("; ")})` : " (trusted)"}`);
  for (const [index, total] of totals.entries()) {
    console.log(`  round ${index + 1}: over ${total.units} units both finished: SQL ${seconds(total.sqlCpuMs)} CPU, ${seconds(total.sqlWallMs)} wall; native ${seconds(total.nativeCpuMs)} CPU, ${seconds(total.nativeWallMs)} wall; TIMEOUT and ERROR units ${seconds(total.timeoutCpuMs)} CPU; round ${seconds(rounds[index]!.wallMs)} wall over ${options.jobs} jobs`);
  }
  console.log(`  SQL/native: CPU ${ratio.cpu.toFixed(2)}×, wall ${ratio.wall.toFixed(2)}× (medians of the rounds' sums); load during SQL units median ${median(unitLoads).toFixed(1)}, max ${Math.max(0, ...unitLoads).toFixed(1)}`);
  const lastOutcomes = new Map<string, UnitOutcome>();
  for (const round of rounds) for (const outcome of round.sql) lastOutcomes.set(unitKey(outcome.unit), outcome);
  const okOutcomes = [...lastOutcomes.values()].filter((outcome): outcome is Extract<UnitOutcome, { status: "ok" }> => outcome.status === "ok");
  if (notCompared.length > 0) {
    console.log("  TIMEOUT and ERROR rows (SQL; CPU and wall time when killed):");
    for (const outcome of [...lastOutcomes.values()]) if (outcome.status !== "ok") console.log(`    ${outcome.status.toUpperCase().padEnd(7)} ${(outcome.cpuMs === null ? "?" : seconds(outcome.cpuMs)).padStart(9)} CPU ${seconds(outcome.wallMs).padStart(9)} wall  ${unitKey(outcome.unit)}: ${outcome.detail}`);
  }
  // Statements, clauses, (clause, file)s: the last round each unit finished in.
  const statements = new Map<string, { milliseconds: number; rows: number; runs: number; units: Set<string> }>();
  for (const outcome of okOutcomes) {
    for (const statement of outcome.statements) {
      const found = statements.get(statement.sql) ?? { milliseconds: 0, rows: 0, runs: 0, units: new Set<string>() };
      statements.set(statement.sql, { milliseconds: found.milliseconds + statement.milliseconds, rows: found.rows + statement.rows, runs: found.runs + statement.runs, units: found.units.add(unitKey(outcome.unit)) });
    }
  }
  const topStatements = [...statements].map(([sql, entry]) => ({ sql, milliseconds: entry.milliseconds, rows: entry.rows, runs: entry.runs, units: [...entry.units].slice(0, 3) })).sort((a, b) => b.milliseconds - a.milliseconds);
  const clauseTimes = names.map(clause => {
    const perRound = rounds.flatMap(round => round.sql.flatMap(outcome => (outcome.status === "ok" ? outcome.clauses.filter(entry => entry.clause === clause).map(entry => entry.sqlMs ?? 0) : outcome.unit.clauses.length === 1 && outcome.unit.clauses[0] === clause ? [outcome.cpuMs ?? outcome.wallMs] : [])));
    return { clause, milliseconds: median(perRound), timeout: !sqlLines.has(clause) };
  }).filter(entry => !Number.isNaN(entry.milliseconds)).sort((a, b) => b.milliseconds - a.milliseconds);
  const clauseFiles = okOutcomes.flatMap(outcome => outcome.clauseFiles).sort((a, b) => b.milliseconds - a.milliseconds);
  console.log("  top statements by SQL time (Σ over units and workers, last round):");
  for (const statement of topStatements.slice(0, 10)) console.log(`    ${seconds(statement.milliseconds).padStart(9)} ${String(statement.rows).padStart(10)} rows ${String(statement.runs).padStart(6)} runs  [${statement.units[0]}] ${statement.sql.replace(/\s+/g, " ").slice(0, 140)}`);
  console.log("  top clauses by SQL time (statement time attributed to the clause, median over rounds; TIMEOUT: its CPU time):");
  for (const entry of clauseTimes.slice(0, options.show)) console.log(`    ${seconds(entry.milliseconds).padStart(9)}${entry.timeout ? " TIMEOUT" : ""}  ${entry.clause}`);
  console.log("  top (clause, file) by SQL time (a batch's time split evenly over its files):");
  for (const entry of clauseFiles.slice(0, 10)) console.log(`    ${seconds(entry.milliseconds).padStart(9)}  ${entry.clause} @ ${entry.path}`);
  // Where SQL's excess over native goes: the last round's units both engines finished, most SQL − native CPU first.
  const lastRound = rounds.at(-1)!;
  const nativeDone = new Map(lastRound.native.flatMap(outcome => (outcome.status === "ok" ? [[unitKey(outcome.unit), outcome] as const] : [])));
  const excess = lastRound.sql.flatMap(outcome => {
    const native = nativeDone.get(unitKey(outcome.unit));
    return outcome.status === "ok" && native !== undefined ? [{ key: unitKey(outcome.unit), sql: outcome, native, excessMs: outcome.cpuMs - native.cpuMs }] : [];
  }).sort((a, b) => b.excessMs - a.excessMs);
  const ms = (value: number | null) => (value === null ? "—" : (value / 1000).toFixed(2)).padStart(7);
  console.log("  top excess by phase (s; SQL engine timers are wall summed over its workers; rest = SQL apply − setup − statements − facts):");
  console.log(`    ${["excess", "SQL", "native", "rules", "lower", "apply", "setup", "stmts", "facts", "rest", "nat.app", "sess", "run", "skip"].map(text => text.padStart(7)).join(" ")}  unit`);
  for (const row of excess.slice(0, 15)) {
    const phases: Phases = row.sql.phases;
    const rest = phases.applyCpuMs - (phases.sessionSetupMs ?? 0) - (phases.statementMs ?? 0) - (phases.factsMs ?? 0);
    const counts = [phases.sessions, phases.statementsRun, phases.statementsSkipped].map(count => String(count ?? "—").padStart(7));
    console.log(`    ${[row.excessMs, row.sql.cpuMs, row.native.cpuMs, phases.ruleCompileMs, phases.programCpuMs, phases.applyCpuMs, phases.sessionSetupMs, phases.statementMs, phases.factsMs, rest, row.native.phases.applyCpuMs].map(ms).join(" ")} ${counts.join(" ")}  ${row.key}`);
  }
  console.log("    no timer yet (inside `rest`): parse and tape, per-file plan building (`SqlFile::plan`), match rendering; lowering and emission are one figure (`lower`)");

  // Against the previous run on this subset and clause set
  if (previous !== undefined) {
    const before = previous.record;
    console.log("");
    console.log(`AGAINST ${previous.path.replace(homedir(), "~")} (${before.started}):`);
    console.log(`  SQL CPU ${seconds(median(before.totals.sqlCpuMs))} → ${seconds(median(sqlCpu))} (${percent(median(sqlCpu) / median(before.totals.sqlCpuMs) - 1)}); SQL/native CPU ${before.ratio.cpu.toFixed(2)}× → ${ratio.cpu.toFixed(2)}×, wall ${before.ratio.wall.toFixed(2)}× → ${ratio.wall.toFixed(2)}× (over the units both runs' engines finished; the units differ where the chunking moved)`);
    const old = new Map<string, number>();
    for (const round of before.rounds.slice(-1)) {
      for (const record of round.sql) {
        if (record.status === "ok") for (const [clause, milliseconds] of Object.entries(record.sqlMs ?? {})) old.set(clause, milliseconds);
        else old.set(record.unit.clauses[0]!, record.cpuMs ?? record.wallMs);
      }
    }
    const deltas = clauseTimes.map(entry => ({ ...entry, before: old.get(entry.clause) ?? 0 })).map(entry => ({ ...entry, delta: entry.milliseconds - entry.before })).sort((a, b) => Math.abs(b.delta) - Math.abs(a.delta));
    console.log("  largest clause changes:");
    for (const entry of deltas.slice(0, 8)) console.log(`    ${entry.delta >= 0 ? "+" : ""}${seconds(entry.delta).padStart(9)}  ${entry.clause} (${seconds(entry.before)} → ${seconds(entry.milliseconds)})`);
    const wasTimeout = new Set(before.rounds.flatMap(round => round.sql.flatMap(record => (record.status === "ok" ? [] : record.unit.clauses))));
    const newTimeouts = notCompared.filter(clause => !wasTimeout.has(clause));
    if (newTimeouts.length > 0) console.log(`  newly not compared (TIMEOUT or ERROR): ${newTimeouts.join(", ")}`);
  }

  // Projection: native's whole tree times the subset's SQL/native CPU ratio, corrected by the calibration.
  const correction = subset.calibration?.correction ?? 1;
  const projectedRatio = ratio.cpu * correction;
  const projection = { ratio: projectedRatio, correction, error: subset.calibration?.error ?? null, nativeWholeWallMs: cache.whole.full.wallMs, sqlWholeWallMs: cache.whole.full.wallMs * projectedRatio };
  console.log("");
  console.log(`PROJECTION: whole-tree SQL ≈ ${seconds(projection.sqlWholeWallMs)} wall = native ${seconds(cache.whole.full.wallMs)} × ${projectedRatio.toFixed(2)}${subset.calibration === null ? " — UNCALIBRATED" : ` (subset ratio × correction ${correction.toFixed(2)}, error ±${(subset.calibration.error * 100).toFixed(0)}%, calibrated ${subset.calibration.date})`}; target ≤ 1.10× = ${seconds(cache.whole.full.wallMs * 1.1)}${notCompared.length > 0 ? `; ${notComparedText} clauses not in it (a lower bound)` : ""}; the whole-program setup is not in it (PROGRAM)`);
  if (program !== undefined) {
    console.log("");
    console.log(program.status === "ok"
      ? `PROGRAM: ${program.clauses} clauses in one SQL program: rules compiled in ${seconds(program.compileMs)}; a first batch per worker (SQL program compile and every session's tables) ${seconds(program.setupMs)}; ${seconds(program.cpuMs)} CPU in all, ${gib(program.rssBytes)} resident at the peak`
      : program.status === "timeout"
        ? `PROGRAM TIMEOUT: compiling every clause into one SQL program and opening the sessions passed the ${options.programCapSeconds} s CPU cap (${seconds(program.cpuMs)} CPU, ${seconds(program.wallMs)} wall, ${gib(program.rssBytes)} resident by then); the whole-tree run pays this before any file`
        : `PROGRAM ERROR: compiling every clause into one SQL program and opening the sessions failed after ${seconds(program.wallMs)}`);
  }

  // Record
  const record = {
    format: "rerail.sql-loop-run/v4" as const,
    started: startedAt.toISOString(),
    turnaroundMs: performance.now() - started,
    machine: { cpus: cpus().length, model: cpus()[0]?.model, memoryBytes: totalmem(), platform: `${process.platform} ${process.arch}`, node: process.version },
    git: { head: git(rerailRoot, "rev-parse", "HEAD").trim(), dirty: git(rerailRoot, "status", "--porcelain", "--untracked-files=no", "--", "native", "Cargo.toml").split("\n").filter(line => line.length > 0) },
    addon: pool.ready?.addon ?? engine.addon,
    subsetSha256: subsetSha(subset),
    subsetFiles: subset.files.length,
    clauseSetSha256: clauseSetSha,
    clauseCount: names.length,
    nativeCache: { path: cachePathOf(key), key, measured: cache.measured, measuredNow: measured },
    options: { rounds: options.rounds, jobs: options.jobs, memoryGib: options.memoryGib, chunkSeconds: options.chunkSeconds, capSeconds: options.capSeconds, groupCapSeconds: options.groupCapSeconds, clauses: options.clauses, programCapSeconds: options.programCapSeconds },
    plan: { units: planned.length, alone: planned.filter(unit => unit.unit.clauses.length === 1).length, costRuns: model.runs, overheadMs: model.overheadMs, estimateMs: { sql: sum(planned.map(unit => unit.estimateMs.sql)), native: sum(planned.map(unit => unit.estimateMs.native)) } },
    load: { background: backgroundLoad, unitsMedian: median(unitLoads), unitsMax: Math.max(0, ...unitLoads) },
    rounds: rounds.map(round => ({ sql: round.sql.map(unitRecord), native: round.native.map(unitRecord), wallMs: round.wallMs, peakRunning: round.peakRunning })),
    totals: { sqlCpuMs: sqlCpu, sqlWallMs: sqlWall, nativeCpuMs: nativeCpu, nativeWallMs: nativeWall },
    ratio,
    parity: { equal: differing.length === 0, compared: parities.length, differing, timeouts, errors, refused: [...refused].map(([clause, reason]) => ({ clause, reason })) },
    statements: topStatements.slice(0, 50),
    clauses: clauseTimes,
    clauseFiles: clauseFiles.slice(0, 200),
    projection,
    program: program ?? null,
    trusted: untrusted.length === 0,
    untrusted,
    problems,
  } satisfies RunRecord;
  mkdirSync(resolve(stateRoot, "loop-runs"), { recursive: true });
  const recordPath = resolve(stateRoot, "loop-runs", `${stamp}.json`);
  writeFileSync(recordPath, `${JSON.stringify(record)}\n`);
  console.log("");
  for (const problem of problems.slice(0, options.show)) console.log(`PROBLEM: ${problem}`);
  if (problems.length > options.show) console.log(`PROBLEM: … ${problems.length - options.show} more`);
  const verdict = problems.length > 0 ? "ERROR" : differing.length > 0 ? "FAIL" : "PASS";
  console.log(`VERDICT ${verdict}: parity ${differing.length === 0 ? "ok" : `${differing.length} clauses differ`}${notCompared.length > 0 ? `, ${notComparedText}` : ""}; SQL/native CPU ${ratio.cpu.toFixed(2)}×, wall ${ratio.wall.toFixed(2)}×${untrusted.length > 0 ? " (untrusted timing)" : ""}; projected whole tree ${projectedRatio.toFixed(2)}×; ${seconds(performance.now() - started)} for the run; record ${recordPath.replace(homedir(), "~")}`);
  return verdict === "PASS" ? 0 : verdict === "FAIL" ? 1 : 3;
}

// ---------------------------------------------------------------------------------------------------------------------
// calibrate: the latest run's units over the full corpus, SQL and native, against the subset's ratio

const calibrationDataSchema = z.object({
  format: z.literal("rerail.sql-loop-calibration/v2"),
  addon: z.string(),
  measured: z.string(),
  /** Per unit key: its clauses, and SQL and native over the full corpus. */
  units: z.record(z.string(), z.object({ unit: unitSchema, sql: unitRecordSchema, native: unitRecordSchema.nullable() })),
});

async function calibrate(options: Options): Promise<number> {
  const subset = readSubset(options.subset);
  const engine = loadEngine(options.addon);
  // The subset's times: the latest run of this addon on this subset.
  const latest = runRecords().find(({ record }) => record.addon.sha256 === engine.addon.sha256 && record.subsetSha256 === subsetSha(subset));
  if (latest === undefined) throw new UsageError(`no loop run of ${engine.addon.sha256} on this subset: \`loop.ts run --addon ${options.addon}\` first`);
  const lastRound = latest.record.rounds.at(-1)!;
  const subsetSql = new Map(latest.record.rounds.flatMap(round => round.sql).filter(record => record.status === "ok").map(record => [unitKey(record.unit), record]));
  const subsetNative = new Map(lastRound.native.filter(record => record.status === "ok").map(record => [unitKey(record.unit), record]));
  const units = [...subsetSql.values()].map(record => record.unit).filter(unit => subsetNative.has(unitKey(unit)) && (options.clauses.length === 0 || unit.clauses.some(clause => options.clauses.some(filter => clause.includes(filter)))));
  const dataPath = resolve(stateRoot, "loop-calibration", `${engine.addon.sha256.slice("sha256:".length, "sha256:".length + 16)}.json`);
  const data = existsSync(dataPath) ? calibrationDataSchema.parse(JSON.parse(readFileSync(dataPath, "utf8"))) : { format: "rerail.sql-loop-calibration/v2" as const, addon: engine.addon.sha256, measured: "", units: {} };
  const missing = units.filter(unit => data.units[unitKey(unit)] === undefined);
  // Longest first by the subset's SQL CPU.
  const estimateOf = (unit: Unit) => subsetSql.get(unitKey(unit))!.cpuMs ?? 0;
  console.log(`calibrate: ${units.length} units of ${latest.path.replace(homedir(), "~")}; ${missing.length} to run over the full corpus (${options.jobs} jobs, CPU caps ${options.capSeconds} s alone, ${options.groupCapSeconds} s per chunk); load ${loadavg()[0]!.toFixed(1)}`);
  if (missing.length > 0) {
    const pool = new UnitPool(options, "full");
    const flush = (sqlOutcomes: readonly UnitOutcome[], nativeOutcomes: readonly UnitOutcome[]) => {
      const nativeByKey = new Map(nativeOutcomes.map(outcome => [unitKey(outcome.unit), outcome]));
      for (const outcome of sqlOutcomes) {
        const native = nativeByKey.get(unitKey(outcome.unit));
        data.units[unitKey(outcome.unit)] = { unit: outcome.unit, sql: unitRecord(outcome), native: native === undefined ? null : unitRecord(native) };
      }
      data.measured = new Date().toISOString();
      mkdirSync(resolve(stateRoot, "loop-calibration"), { recursive: true });
      writeFileSync(dataPath, `${JSON.stringify(data)}\n`);
    };
    // A chunk past its cap is one TIMEOUT unit: it is left out of the calibration.
    const sql = await runUnits(pool, { items: missing.map(unit => ({ kind: "unit", engine: "sql", unit, estimateMs: estimateOf(unit) })), capSeconds: options.capSeconds, groupCapSeconds: options.groupCapSeconds, jobs: options.jobs, label: "sql full", onUnit: (_pairs, outcomes) => flush(outcomes, []) });
    const finished = sql.outcomes.filter(outcome => outcome.status === "ok" && missing.some(unit => unitKey(unit) === unitKey(outcome.unit))).map(outcome => outcome.unit);
    const native = await runUnits(pool, { items: finished.map(unit => ({ kind: "unit", engine: "native", unit, estimateMs: estimateOf(unit) })), capSeconds: options.capSeconds, groupCapSeconds: options.groupCapSeconds, jobs: options.jobs, label: "native full" });
    pool.close();
    flush(sql.outcomes.filter(outcome => missing.some(unit => unitKey(unit) === unitKey(outcome.unit))), native.outcomes);
  }
  const rows = units.flatMap(unit => {
    const full = data.units[unitKey(unit)];
    if (full === undefined) return [];
    const sub = { sql: subsetSql.get(unitKey(unit))!.cpuMs!, native: subsetNative.get(unitKey(unit))!.cpuMs! };
    return [{ key: unitKey(unit), unit, policy: unit.linter, ok: full.sql.status === "ok" && full.native?.status === "ok", sub, full: { sql: full.sql.cpuMs ?? full.sql.wallMs, native: full.native?.cpuMs ?? null } }];
  });
  const calibrated = rows.filter(row => row.ok);
  const ratioOf = (selected: typeof calibrated, side: "sub" | "full") => sum(selected.map(row => row[side].sql)) / sum(selected.map(row => row[side].native!));
  const subsetRatio = ratioOf(calibrated, "sub");
  const fullRatio = ratioOf(calibrated, "full");
  const correction = fullRatio / subsetRatio;
  // Each policy's full-corpus SQL time, projected from its subset ratio with the correction the other policies give.
  const policies = [...new Set(calibrated.map(row => row.policy))];
  const parts = policies.map(policy => {
    const own = calibrated.filter(row => row.policy === policy);
    const others = calibrated.filter(row => row.policy !== policy);
    const factor = others.length === 0 ? 1 : ratioOf(others, "full") / ratioOf(others, "sub");
    return { policy, actualMs: sum(own.map(row => row.full.sql)), projectedMs: ratioOf(own, "sub") * factor * sum(own.map(row => row.full.native!)), subsetRatio: ratioOf(own, "sub"), fullRatio: ratioOf(own, "full") };
  });
  const error = sum(parts.map(part => Math.abs(part.projectedMs - part.actualMs))) / sum(parts.map(part => part.actualMs));
  const calibration: Calibration = {
    date: new Date().toISOString(),
    addon: engine.addon.sha256,
    head: git(rerailRoot, "rev-parse", "HEAD").trim(),
    correction,
    error,
    totalError: subsetRatio / fullRatio - 1,
    units: calibrated.length,
    timeouts: rows.filter(row => !row.ok).map(row => row.key),
    note: `the units of the run ${latest.record.started} over the full corpus (caps ${options.capSeconds} s alone, ${options.groupCapSeconds} s per chunk): SQL/native CPU ${fullRatio.toFixed(3)} there against ${subsetRatio.toFixed(3)} on the subset`,
    subsetRatio,
    fullRatio,
    policies: parts,
  };
  writeFileSync(options.subset, `${JSON.stringify({ ...readSubset(options.subset), calibration }, null, 2)}\n`);
  console.log(`calibration over ${calibrated.length} units (${calibration.timeouts.length} past their cap on the full corpus): SQL/native CPU ${subsetRatio.toFixed(3)}× on the subset, ${fullRatio.toFixed(3)}× on the full corpus (${percent(calibration.totalError)}); correction ${correction.toFixed(3)}; error ±${(error * 100).toFixed(1)}% (each policy corrected by the others)`);
  console.log("  policy                              subset ratio  full ratio   full SQL    projected");
  for (const part of parts.sort((a, b) => b.actualMs - a.actualMs)) console.log(`  ${part.policy.padEnd(34)} ${part.subsetRatio.toFixed(2).padStart(11)}× ${part.fullRatio.toFixed(2).padStart(10)}× ${seconds(part.actualMs).padStart(10)} ${seconds(part.projectedMs).padStart(12)}`);
  console.log("  units: subset SQL / native → full SQL / native (CPU)");
  for (const row of [...rows].sort((a, b) => b.full.sql - a.full.sql).slice(0, options.show)) console.log(`  ${seconds(row.sub.sql).padStart(9)} / ${seconds(row.sub.native).padStart(8)} → ${seconds(row.full.sql).padStart(9)}${row.ok ? " " : "+"} / ${row.full.native === null ? "?" : seconds(row.full.native)}  ${row.key}`);
  if (calibration.timeouts.length > 0) console.log(`  past the cap on the full corpus: ${calibration.timeouts.join(", ")}`);
  if (options.output !== undefined) writeFileSync(options.output, `${JSON.stringify({ calibration, rows }, null, 2)}\n`);
  console.log(`  wrote the calibration into ${options.subset}; data ${dataPath.replace(homedir(), "~")}`);
  return 0;
}

// ---------------------------------------------------------------------------------------------------------------------
// costs: each file's SQL time with every clause in one program, one file per apply, each capped

const costsFileSchema = z.object({
  format: z.literal("rerail.sql-loop-costs/v3"),
  addon: z.string(),
  measured: z.string(),
  capSeconds: z.number(),
  /** Per file: its single-file apply wall time with every clause; a capped one holds the cap. Costliest first. */
  files: z.array(z.object({ path: z.string(), milliseconds: z.number(), capped: z.boolean() })),
  /** Per (clause, file): the statement time the profile gives the clause on the file (files that finished). */
  pairs: z.array(z.object({ clause: z.string(), path: z.string(), milliseconds: z.number() })),
});
type CostsFile = z.infer<typeof costsFileSchema>;

/**
 * Times each file of the subset (or `--files-from`) on SQL with every clause in one program (production's shape), one
 * file per apply in batches of one, each capped at `--cap`: a capped file is skipped and its child replaced. The files
 * are dealt out to `--jobs` children, largest first; the costs file is rewritten as files finish.
 */
async function costs(options: Options): Promise<number> {
  const started = performance.now();
  const engine = loadEngine(options.addon);
  const output = options.output ?? defaultCosts;
  const paths = options.filesFrom === undefined ? readSubset(options.subset).files.map(file => file.path) : readFileSync(options.filesFrom, "utf8").split("\n").filter(path => path.length > 0);
  const root = realpathSync(options.corpus);
  const bytes = new Map(paths.map(path => [path, readFileSync(resolve(root, path)).length]));
  const ordered = [...paths].sort((a, b) => bytes.get(b)! - bytes.get(a)! || compareStrings(a, b));
  const items = Array.from({ length: Math.min(options.jobs, ordered.length) }, (_, job): QueueItem => ({ kind: "unit", engine: "sql", unit: { linter: ALL, clauses: [] }, files: ordered.filter((_, index) => index % options.jobs === job), estimateMs: 0 }));
  console.log(`costs: ${paths.length} files, every clause in one SQL program, ${items.length} jobs, ${options.capSeconds} s CPU cap per file (${options.groupCapSeconds} s for the program setup); load ${loadavg()[0]!.toFixed(1)}`);
  const write = (pairs: readonly PairCost[]) => {
    const file: CostsFile = {
      format: "rerail.sql-loop-costs/v3", addon: engine.addon.sha256, measured: new Date().toISOString(), capSeconds: options.capSeconds,
      files: pairs.filter(pair => pair.clause === ALL).map(({ path, milliseconds, capped }) => ({ path, milliseconds, capped })).sort((a, b) => b.milliseconds - a.milliseconds),
      pairs: pairs.filter(pair => pair.clause !== ALL).map(({ clause, path, milliseconds }) => ({ clause, path, milliseconds })).sort((a, b) => b.milliseconds - a.milliseconds),
    };
    writeFileSync(output, `${JSON.stringify(file)}\n`);
    return file;
  };
  const pool = new UnitPool(options, options.filesFrom === undefined ? "subset" : "list", options.filesFrom);
  const result = await runUnits(pool, { items, capSeconds: options.capSeconds, groupCapSeconds: options.groupCapSeconds, jobs: options.jobs, perFile: true, label: "costs", onUnit: pairs => write(pairs) });
  pool.close();
  const file = write(result.pairs);
  const capped = file.files.filter(entry => entry.capped);
  console.log(`costs: ${file.files.length} files in ${seconds(performance.now() - started)}; Σ ${seconds(sum(file.files.map(entry => entry.milliseconds)))} single-file SQL wall (capped files at the cap), ${capped.length} capped at ${options.capSeconds} s`);
  for (const entry of file.files.slice(0, 25)) console.log(`  ${(entry.capped ? `>${options.capSeconds} s` : seconds(entry.milliseconds)).padStart(9)}  ${entry.path}`);
  console.log("  costliest (clause, file)s:");
  for (const pair of file.pairs.slice(0, 15)) console.log(`  ${seconds(pair.milliseconds).padStart(9)}  ${pair.clause} @ ${pair.path}`);
  console.log(`  wrote ${output}`);
  return 0;
}

// ---------------------------------------------------------------------------------------------------------------------
// select

const distilledSchema = z.object({
  sources: z.array(z.string()).default([]),
  files: z.array(z.object({ path: z.string(), why: z.array(z.string()) })),
});

/**
 * Writes the subset manifest from a distilled subset (`--from`, the distillation pool's `subset.json`: each file with
 * why it is there). Native runs over the subset alone (the oracle; cross-file clauses depend on which files are
 * present) and over the full tree; per clause the manifest records a subset file native reports it on, a clean one in
 * its scope, and the finding counts.
 */
async function select(options: Options): Promise<number> {
  if (options.from === undefined) throw new UsageError("select needs --from <distilled subset.json>");
  const source = readFileSync(options.from);
  const distilled = distilledSchema.parse(JSON.parse(source.toString("utf8")));
  const engine = loadEngine(options.addon);
  const rules = loadRules(engine, options.policies);
  const repository = collectRepository(engine, options.corpus);
  const byPath = new Map(repository.files.map(file => [file.path, file]));
  const missing = distilled.files.filter(file => !byPath.has(file.path));
  if (missing.length > 0) throw new UsageError(`${missing.length} distilled files are not corpus Python files: ${missing.slice(0, 5).map(file => file.path).join(", ")}`);
  const paths = distilled.files.map(file => file.path).sort(compareStrings);
  const linterOf = new Map(rules.map(rule => [documentSchema.parse(rule.document).id, rule.linter]));
  const findingsOver = async (files: readonly SourceFile[]) => {
    const workspace = new RuleWorkspace(engine, rules, { engine: "native" });
    try {
      const timed = await workspace.apply(files.filter(file => workspace.accepts(file.path)));
      const found = new Map<string, Map<string, number>>();
      for (const violation of timed.result.violations) {
        const name = `${linterOf.get(violation.ruleId) ?? violation.ruleId}/${violation.clauseId}`;
        const perPath = found.get(name) ?? new Map<string, number>();
        found.set(name, perPath);
        perPath.set(violation.path ?? "", (perPath.get(violation.path ?? "") ?? 0) + 1);
      }
      return { found, wallMs: timed.wallMs };
    } finally {
      workspace.close();
    }
  };
  const full = await findingsOver(repository.files);
  const sub = await findingsOver(paths.map(path => byPath.get(path)!));
  const coverage: Subset["clauses"] = {};
  for (const rule of rules) {
    const workspace = new RuleWorkspace(engine, [rule], { engine: "native" });
    const inScope = paths.filter(path => workspace.accepts(path));
    workspace.close();
    for (const clause of reportingClauses([rule])) {
      const hits = sub.found.get(clause.name) ?? new Map<string, number>();
      const fullFindings = sum(full.found.get(clause.name)?.values() ?? []);
      const finding = [...hits.keys()].sort(compareStrings)[0] ?? null;
      const clean = inScope.find(path => !hits.has(path)) ?? null;
      coverage[clause.name] = {
        finding, clean, subsetFindings: sum(hits.values()), fullFindings, crossFile: clause.crossFile,
        status: fullFindings === 0 ? "no-finding-in-full-tree" : finding === null ? "finding-only-in-full-run" : clean === null ? "no-clean-file" : "covered",
      };
    }
  }
  const previous = existsSync(options.subset) ? subsetSchema.safeParse(JSON.parse(readFileSync(options.subset, "utf8"))) : undefined;
  const why = new Map(distilled.files.map(file => [file.path, file.why]));
  const files = paths.map(path => ({ path, bytes: Buffer.byteLength(byPath.get(path)!.source), reasons: [...why.get(path)!].sort(compareStrings) }));
  const manifest: Subset = {
    format: "rerail.sql-loop-subset/v3",
    generated: new Date().toISOString(),
    corpus: { commit: repository.git.commit, files: repository.files.length, bytes: repository.bytes },
    policies: policiesHash(options.policies),
    source: { path: options.from.replace(homedir(), "~"), sha256: sha256(source), sources: distilled.sources },
    files,
    clauses: coverage,
    // A changed file list invalidates the calibration.
    calibration: previous?.success === true && JSON.stringify(previous.data.files.map(file => file.path)) === JSON.stringify(paths) ? previous.data.calibration : null,
  };
  writeFileSync(options.subset, `${JSON.stringify(manifest, null, 2)}\n`);
  const statuses = new Map<string, number>();
  for (const entry of Object.values(coverage)) statuses.set(entry.status, (statuses.get(entry.status) ?? 0) + 1);
  console.log(`select: ${files.length} files (${(sum(files.map(file => file.bytes)) / 1e6).toFixed(2)} MB of ${(repository.bytes / 1e6).toFixed(1)} MB) from ${options.from}; native full ${seconds(full.wallMs)}, subset ${seconds(sub.wallMs)}`);
  console.log(`  clauses: ${[...statuses].map(([status, count]) => `${count} ${status}`).join(", ")}`);
  console.log(`  wrote ${options.subset}${manifest.calibration === null ? " (uncalibrated: run `loop.ts run`, then `loop.ts calibrate`)" : ""}`);
  return 0;
}

// ---------------------------------------------------------------------------------------------------------------------

async function main(): Promise<number> {
  let options: Options;
  try {
    options = parseOptions(process.argv.slice(2));
    // The addon's worker threads can outlive a whole-program workspace: a child exits once its requests end.
    if (options.command === "unit-child") process.exit(await unitChild(options));
    if (options.command === "program-child") return await programChild(options);
    if (!existsSync(options.addon)) throw new UsageError(`no addon at ${options.addon}; build it with \`just rerail-sql-loop\` (or --addon)`);
    return options.command === "select" ? await select(options) : options.command === "costs" ? await costs(options) : options.command === "calibrate" ? await calibrate(options) : await run(options);
  } catch (error) {
    if (!(error instanceof UsageError)) throw error;
    process.stderr.write(`rerail-sql-loop: ${error.message}\n`);
    return 2;
  }
}

if (resolve(process.argv[1] ?? "") === script) process.exitCode = await main();
