// `just rerail-sql-loop`: the SQL engine's validation loop. Every reporting clause runs alone (with its policy's
// `define`s, as the census runs it) over a checked-in subset of the corpus (`loop-subset.json`), on native and on SQL,
// in alternating rounds spread over `--jobs` child processes. A clause past its cap is killed and reported as a
// TIMEOUT row with its time. The verdict is parity (each clause's SQL lines against native's, apart from native's
// budget failures), speed (summed per-clause wall and CPU, SQL/native ratio, the costliest statements, clauses and
// (clause, file)s, against the previous run), a per-clause projection to the full corpus, and the whole-program SQL
// setup (every clause compiled into one program and a first session), capped. Native's per-clause lines, subset times
// and full-corpus times are cached per native source hash (`~/rerail-bench/data-layer/loop-cache/`); every run is
// recorded in `~/rerail-bench/data-layer/loop-runs/<timestamp>.json`.
//
//   loop.ts run        [--rounds N] [--jobs N] [--cap S] [--native fresh|cached] [--clauses a,b] [--program-cap S] [--show N]
//   loop.ts calibrate  [--jobs N] [--cap S] [--clauses a,b]: SQL per clause on the full corpus against the subset's projection
//   loop.ts costs      [--jobs N] [--cap S] [--clauses a,b] [--files-from list]: per-(clause, file) SQL times, one file per apply
//   loop.ts select     [--seed N] [--sample N] [--costs json] [--sql-budget S] [--file-limit S]: writes loop-subset.json
//   loop.ts unit-child | program-child (internal): one JSON line per event on stdout
import { type ChildProcess, spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
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
/** Statements and (clause, file)s a unit reports: its costliest. */
const UNIT_STATEMENTS = 20;
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
  native: "fresh" | "cached";
  jobs: number;
  /** The per-unit cap in seconds: per clause (run, calibrate) or per (clause, file) (costs). */
  capSeconds: number;
  nativeCapSeconds: number;
  programCapSeconds: number;
  /** Clause filters: a clause runs where its `linter/clause` name holds any of them. */
  clauses: string[];
  show: number;
  seed: number;
  sample: number;
  costs: string;
  sqlBudgetSeconds: number;
  fileLimitSeconds: number;
  /** costs: a clause stops after this many capped files. */
  maxCapped: number;
  filesFrom: string | undefined;
  output: string | undefined;
  // unit children only
  scope: "subset" | "full" | "list";
}

const valued = [
  "--addon", "--corpus", "--policies", "--subset", "--rounds", "--native", "--jobs", "--cap", "--native-cap", "--program-cap",
  "--clauses", "--show", "--seed", "--sample", "--costs", "--sql-budget", "--file-limit", "--max-capped", "--files-from", "--output", "--scope",
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
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index]!;
    const [key, inline] = argument.includes("=") ? [argument.slice(0, argument.indexOf("=")), argument.slice(argument.indexOf("=") + 1)] : [argument, undefined];
    if (!valued.includes(key)) throw new UsageError(`unknown option ${argument}`);
    const value = inline ?? args[++index];
    if (value === undefined) throw new UsageError(`${key} needs a value`);
    values[key] = value;
  }
  const native = values["--native"] ?? "fresh";
  if (native !== "fresh" && native !== "cached") throw new UsageError(`--native takes fresh or cached, not ${native}`);
  const scope = values["--scope"] ?? "subset";
  if (scope !== "subset" && scope !== "full" && scope !== "list") throw new UsageError(`--scope takes subset, full or list, not ${scope}`);
  const defaultCap = command === "calibrate" ? "120" : command === "costs" ? "20" : "30";
  return {
    command: command as Options["command"],
    addon: resolve(values["--addon"] ?? process.env.RERAIL_LOOP_ADDON ?? defaultAddon),
    corpus: resolve(values["--corpus"] ?? process.env.RERAIL_DATA_LAYER_CORPUS ?? resolve(benchHome, "nera-data-layer-fce4dc72")),
    policies: resolve(values["--policies"] ?? process.env.RERAIL_DATA_LAYER_POLICIES ?? resolve(stateRoot, "policies")),
    subset: resolve(values["--subset"] ?? defaultSubset),
    rounds: integer("--rounds", values["--rounds"] ?? "2", 1),
    native,
    jobs: integer("--jobs", values["--jobs"] ?? "3", 1),
    capSeconds: integer("--cap", values["--cap"] ?? defaultCap, 1),
    nativeCapSeconds: integer("--native-cap", values["--native-cap"] ?? "120", 1),
    programCapSeconds: integer("--program-cap", values["--program-cap"] ?? "30", 0),
    clauses: (values["--clauses"] ?? "").split(",").filter(filter => filter.length > 0),
    show: integer("--show", values["--show"] ?? "15", 0),
    seed: integer("--seed", values["--seed"] ?? "20261019", 0),
    sample: integer("--sample", values["--sample"] ?? "40", 0),
    costs: resolve(values["--costs"] ?? defaultCosts),
    sqlBudgetSeconds: integer("--sql-budget", values["--sql-budget"] ?? "180", 1),
    fileLimitSeconds: integer("--file-limit", values["--file-limit"] ?? "5", 1),
    maxCapped: integer("--max-capped", values["--max-capped"] ?? "2", 1),
    filesFrom: values["--files-from"] === undefined ? undefined : resolve(values["--files-from"]),
    output: values["--output"] === undefined ? undefined : resolve(values["--output"]),
    scope,
  };
}

// ---------------------------------------------------------------------------------------------------------------------
// The subset manifest

const clauseCoverageSchema = z.object({
  /** A subset file native reports a finding of the clause on, from the full tree; null when none exists there. */
  finding: z.string().nullable(),
  /** A subset file in the rule's scope where the clause finds nothing. */
  clean: z.string().nullable(),
  /** Native findings of the clause over the subset, and over the full tree. */
  subsetFindings: z.number().int(),
  fullFindings: z.number().int(),
  /** `select_when`, `require_each` or `budget`: its findings depend on other files. */
  crossFile: z.boolean(),
  status: z.enum(["covered", "no-finding-in-full-tree", "finding-only-in-full-run", "no-clean-file"]),
});
const calibrationSchema = z.object({
  date: z.string(),
  /** The addon (sha256) and HEAD the calibration measured. */
  addon: z.string(),
  head: z.string(),
  /** Σ full-corpus SQL ÷ Σ projected over the calibrated clauses: what a run's projection is multiplied by. */
  correction: z.number(),
  /** The corrected projection's relative error, leaving each policy out of the correction that projects it. */
  error: z.number(),
  /** The uncorrected projection's error: Σ projected ÷ Σ actual − 1, and per clause. */
  totalError: z.number(),
  medianAbsError: z.number(),
  p90AbsError: z.number(),
  clauses: z.number().int(),
  timeouts: z.array(z.string()),
  note: z.string(),
}).passthrough();
const subsetSchema = z.object({
  format: z.literal("rerail.sql-loop-subset/v2"),
  generated: z.string(),
  corpus: z.object({ commit: z.string(), files: z.number().int(), bytes: z.number().int() }),
  policies: z.record(z.string(), z.string()),
  selection: z.object({ seed: z.number().int(), sample: z.number().int(), costs: z.string().nullable(), sqlBudgetSeconds: z.number(), fileLimitSeconds: z.number() }),
  /** `sqlMs`: the file's summed per-clause SQL time in the costs the selection read (estimated where unmeasured). */
  files: z.array(z.object({ path: z.string(), bytes: z.number().int(), sqlMs: z.number().optional(), reasons: z.array(z.string()) })),
  /** The subset's summed per-(clause, file) SQL time in those costs. */
  sqlCostMs: z.number().nullable(),
  /** Files the selection would have taken but left out for their SQL cost: validated only in the full run. */
  excluded: z.array(z.object({ path: z.string(), reason: z.string() })),
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
// Clauses: each reporting clause alone, with the `define`s it may read

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

/** `<policies>/<linter>/policy.json` with only `clauseId` and the policy's `define`s, compiled. */
function clauseRule(engine: Engine, policies: string, name: string): PortedRule {
  const linter = name.slice(0, name.indexOf("/"));
  const clauseId = name.slice(name.indexOf("/") + 1);
  const path = resolve(policies, linter, "policy.json");
  if (!existsSync(path)) throw new Error(`no policy for ${name}: ${path}`);
  // From the file: `PortedRule.document` keeps only a policy's id and clauses.
  const document = documentSchema.parse(JSON.parse(readFileSync(path, "utf8")));
  const clauses = [...document.clauses.filter(clause => clause.define !== undefined), ...document.clauses.filter(clause => clause.id === clauseId && clause.define === undefined)];
  const source = JSON.stringify({ ...document, clauses });
  return { linter, path, sha256: sha256(source), clauses: clauses.length, document: { ...document, clauses }, compiled: engine.binding.CodeRuleDslRuntime.compile(source, "json", engine.capabilities.source) };
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
// Unit children: clauses alone, one at a time, on requests from stdin

const statementSchema = z.object({ sql: z.string(), milliseconds: z.number(), rows: z.number(), runs: z.number() });
const clauseFileSchema = z.object({ path: z.string(), milliseconds: z.number() });
const requestSchema = z.object({
  id: z.number().int(),
  engine: z.enum(["native", "sql"]),
  clause: z.string(),
  /** Only these files of the child's scope (those the clause's rule accepts). */
  files: z.array(z.string()).optional(),
  /** One file per apply, each timed (`costs`); SQL then runs batches of one file. */
  perFile: z.boolean().default(false),
});
type UnitRequest = z.input<typeof requestSchema>;
const unitEventSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("ready"), setupMs: z.number(), files: z.number().int(), addon: z.object({ path: z.string(), sha256: z.string(), kernelVersion: z.string(), work: z.string() }), workerThreads: z.number() }),
  /** The request was read: its accepted files. */
  z.object({ kind: z.literal("start"), id: z.number().int(), files: z.array(z.string()) }),
  /** The untimed warm-up ended: the timed apply starts. */
  z.object({ kind: z.literal("warm"), id: z.number().int(), warmupMs: z.number() }),
  z.object({ kind: z.literal("file-start"), id: z.number().int(), path: z.string() }),
  z.object({ kind: z.literal("file"), id: z.number().int(), path: z.string(), wallMs: z.number(), cpuMs: z.number() }),
  z.object({
    kind: z.literal("unit"),
    id: z.number().int(),
    /** Compiling the clause; then the untimed warm-up: one-statement files at paths the clause accepts, one per worker (on SQL: the program's compile and each worker's session, whose tables it creates). */
    compileMs: z.number(),
    warmupMs: z.number(),
    wallMs: z.number(),
    cpuMs: z.number(),
    load: z.number(),
    files: z.number().int(),
    lines: z.array(z.string()),
    refused: z.array(z.string()),
    statements: z.array(statementSchema),
    clauseFiles: z.array(clauseFileSchema),
  }),
]);
type UnitEvent = z.infer<typeof unitEventSchema>;
type UnitDone = Extract<UnitEvent, { kind: "unit" }>;

async function unitChild(options: Options): Promise<number> {
  const started = performance.now();
  const emit = (event: UnitEvent) => process.stdout.write(`${JSON.stringify(event)}\n`);
  const engine = loadEngine(options.addon);
  if (engine.addon.work !== "unbounded") throw new Error(`${engine.addon.path} is not an unbounded-work build`);
  const files = options.scope === "full"
    ? collectRepository(engine, options.corpus).files
    : readFiles(engine, options.corpus, options.scope === "list" ? readFileSync(options.filesFrom!, "utf8").split("\n").filter(path => path.length > 0) : readSubset(options.subset).files.map(file => file.path));
  const byPath = new Map(files.map(file => [file.path, file]));
  emit({ kind: "ready", setupMs: performance.now() - started, files: files.length, addon: engine.addon, workerThreads: engine.workerThreads });
  for await (const text of createInterface({ input: process.stdin })) {
    if (text.length === 0) continue;
    const request = requestSchema.parse(JSON.parse(text));
    const compileStarted = performance.now();
    const single = clauseRule(engine, options.policies, request.clause);
    const compileMs = performance.now() - compileStarted;
    const matcher: Matcher = request.engine === "sql" ? { engine: "sql", ...(request.perFile ? { sqlBatchSize: 1 } : {}) } : { engine: "native" };
    const workspace = new RuleWorkspace(engine, [single], matcher);
    try {
      const candidates = request.files === undefined ? files : request.files.map(path => byPath.get(path)!);
      const accepted = candidates.filter(file => workspace.accepts(file.path));
      emit({ kind: "start", id: request.id, files: accepted.map(file => file.path) });
      let warmupMs = 0;
      if (accepted.length > 0) {
        // The whole-tree run compiles the program and opens each worker's session once for every clause together;
        // a clause alone pays them here, untimed. A batch per worker (one file each) opens every session.
        const warmStarted = performance.now();
        const dummies = accepted.slice(0, request.perFile ? 1 : engine.workerThreads).map(file => ({ ...file, source: "pass\n" }));
        await workspace.apply(dummies);
        await workspace.apply([], dummies.map(file => file.path));
        warmupMs = performance.now() - warmStarted;
      }
      emit({ kind: "warm", id: request.id, warmupMs });
      const load = loadavg()[0]!;
      let wallMs = 0;
      let cpuMs = 0;
      const lines: string[] = [];
      if (request.perFile) {
        for (const file of accepted) {
          emit({ kind: "file-start", id: request.id, path: file.path });
          const timed = await workspace.apply([file]);
          await workspace.apply([], [file.path]);
          wallMs += timed.wallMs;
          cpuMs += timed.cpuMs;
          lines.push(...linesOf(timed.result));
          emit({ kind: "file", id: request.id, path: file.path, wallMs: timed.wallMs, cpuMs: timed.cpuMs });
        }
      } else if (accepted.length > 0) {
        const timed = await workspace.apply(accepted);
        ({ wallMs, cpuMs } = timed);
        lines.push(...linesOf(timed.result));
      }
      const report = workspace.engineReport();
      const sql = report.engine === "sql" ? report : undefined;
      emit({
        kind: "unit", id: request.id, compileMs, warmupMs, wallMs, cpuMs, load, files: accepted.length, lines: lines.sort(compareStrings),
        refused: sql?.refused.map(refusal => refusal.reason) ?? [],
        statements: sql?.statements.profile?.slice(0, UNIT_STATEMENTS) ?? [],
        clauseFiles: sql?.profile?.clauseFiles.slice(0, UNIT_CLAUSE_FILES).map(({ path, milliseconds }) => ({ path, milliseconds })) ?? [],
      });
    } finally {
      workspace.close();
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

  /** The next event, or undefined once the child exited (or `milliseconds` passed: "timeout"). */
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

type ReadyEvent = Extract<UnitEvent, { kind: "ready" }>;

/** A clause's outcome on one engine: done, or killed at its cap (TIMEOUT, with its time), or its child died. */
type UnitOutcome =
  | ({ status: "ok"; clause: string } & Omit<UnitDone, "kind" | "id">)
  | { status: "timeout" | "error"; clause: string; wallMs: number; detail: string };

interface PairCost {
  clause: string;
  path: string;
  milliseconds: number;
  capped: boolean;
}

/** Unit children over one scope, kept between batches: a child is replaced only when a cap kills it. */
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

interface UnitBatch {
  engine: EngineName;
  clauses: readonly string[];
  capSeconds: number;
  jobs: number;
  perFile?: boolean;
  /** Per file: a clause stops after this many capped files (TIMEOUT, the rest of its files unmeasured). */
  maxCappedFiles?: number;
  /** Called after each clause's unit ends, however it ends. */
  onUnit?: (pairs: readonly PairCost[], outcomes: ReadonlyMap<string, UnitOutcome>) => void;
  label: string;
}

/**
 * Runs each clause of `batch` in one of `jobs` children of `pool`, costliest first as given. A clause past its cap (per
 * clause; per file with `perFile`) kills its child: the clause is a TIMEOUT row (per file: that (clause, file) is
 * capped, and the clause goes on without it), and a new child takes the next clause.
 */
async function runUnits(pool: UnitPool, batch: UnitBatch): Promise<{ outcomes: Map<string, UnitOutcome>; pairs: PairCost[]; wallMs: number }> {
  const started = performance.now();
  const queue: { clause: string; files?: string[] }[] = batch.clauses.map(clause => ({ clause }));
  const outcomes = new Map<string, UnitOutcome>();
  const pairs: PairCost[] = [];
  let nextId = 0;
  let done = 0;
  const capMs = batch.capSeconds * 1000;
  const cappedFiles = new Map<string, number>();
  const worker = async () => {
    let child: UnitChild | undefined;
    for (let item = queue.shift(); item !== undefined; item = queue.shift()) {
      child ??= await pool.acquire();
      const id = nextId++;
      const unitStarted = performance.now();
      child.send({ id, engine: batch.engine, clause: item.clause, ...(item.files === undefined ? {} : { files: item.files }), perFile: batch.perFile ?? false });
      let deadline = unitStarted + capMs;
      let accepted: string[] = [];
      const finished = new Set<string>();
      let current: string | undefined;
      let warmed = false;
      for (;;) {
        const event = await child.next(deadline - performance.now());
        if (event === "timeout" || event === undefined) {
          child.kill();
          child = undefined;
          const elapsed = performance.now() - unitStarted;
          if (event === "timeout" && batch.perFile && current !== undefined) {
            pairs.push({ clause: item.clause, path: current, milliseconds: capMs, capped: true });
            const rest = accepted.filter(path => !finished.has(path) && path !== current);
            const capped = (cappedFiles.get(item.clause) ?? 0) + 1;
            cappedFiles.set(item.clause, capped);
            process.stderr.write(`  ${batch.label}: ${item.clause} @ ${current} passed the ${batch.capSeconds} s cap\n`);
            if (capped < (batch.maxCappedFiles ?? Infinity)) queue.unshift({ clause: item.clause, files: rest });
            else {
              outcomes.set(item.clause, { status: "timeout", clause: item.clause, wallMs: elapsed, detail: `${capped} files past the ${batch.capSeconds} s cap; ${rest.length} files unmeasured` });
              process.stderr.write(`  ${batch.label}: ${item.clause} TIMEOUT: ${capped} files past the cap, ${rest.length} left unmeasured\n`);
            }
          } else {
            const status = event === "timeout" ? "timeout" : "error";
            const phase = warmed ? "" : " in setup (compile and sessions, before any timed file)";
            outcomes.set(item.clause, { status, clause: item.clause, wallMs: elapsed, detail: status === "timeout" ? `killed at the ${batch.capSeconds} s cap${phase}` : `the child exited${phase}` });
            process.stderr.write(`  ${batch.label}: ${item.clause} ${status === "timeout" ? `TIMEOUT at ${seconds(elapsed)}` : "ERROR: its child exited"}${phase}\n`);
          }
          batch.onUnit?.(pairs, outcomes);
          break;
        }
        if (event.kind === "start" && event.id === id) accepted = event.files;
        else if (event.kind === "warm" && event.id === id) warmed = true;
        else if (event.kind === "file-start" && event.id === id) {
          current = event.path;
          deadline = performance.now() + capMs;
        } else if (event.kind === "file" && event.id === id) {
          finished.add(event.path);
          current = undefined;
          pairs.push({ clause: item.clause, path: event.path, milliseconds: event.wallMs, capped: false });
        } else if (event.kind === "unit" && event.id === id) {
          const { kind: _kind, id: _id, ...rest } = event;
          const previous = outcomes.get(item.clause);
          // A per-file clause resumed after a capped file: its times add up over the pieces.
          outcomes.set(item.clause, previous?.status === "ok" && batch.perFile
            ? { ...rest, status: "ok", clause: item.clause, wallMs: previous.wallMs + rest.wallMs, cpuMs: previous.cpuMs + rest.cpuMs, files: previous.files + rest.files, lines: [...previous.lines, ...rest.lines] }
            : { ...rest, status: "ok", clause: item.clause });
          done += 1;
          if (done % 100 === 0) process.stderr.write(`  ${batch.label}: ${done}/${batch.clauses.length} clauses, ${seconds(performance.now() - started)}\n`);
          batch.onUnit?.(pairs, outcomes);
          break;
        }
      }
    }
    if (child !== undefined) pool.release(child);
  };
  await Promise.all(Array.from({ length: Math.min(batch.jobs, Math.max(1, batch.clauses.length)) }, worker));
  return { outcomes, pairs, wallMs: performance.now() - started };
}

// ---------------------------------------------------------------------------------------------------------------------
// The whole-program SQL setup: every clause in one program, compiled, and a first session

const programEventSchema = z.object({ kind: z.literal("program"), compileMs: z.number(), warmupMs: z.number(), clauses: z.number().int() });

async function programChild(options: Options): Promise<number> {
  const engine = loadEngine(options.addon);
  const compileStarted = performance.now();
  const rules = loadRules(engine, options.policies);
  const compileMs = performance.now() - compileStarted;
  const workspace = new RuleWorkspace(engine, rules, { engine: "sql" });
  try {
    const started = performance.now();
    // `serverplan_quality` scopes `**/*.py`: this file reaches the SQL program, which compiles and opens a session.
    await workspace.apply([{ path: "__rerail_sql_loop_program__.py", language: "python", source: "\n" }]);
    const warmupMs = performance.now() - started;
    const event: z.infer<typeof programEventSchema> = { kind: "program", compileMs, warmupMs, clauses: reportingClauses(rules).length };
    process.stdout.write(`${JSON.stringify(event)}\n`);
  } finally {
    workspace.close();
  }
  return 0;
}

type ProgramOutcome = { status: "ok"; compileMs: number; warmupMs: number; clauses: number } | { status: "timeout" | "error"; wallMs: number };

async function runProgram(options: Options): Promise<ProgramOutcome> {
  const started = performance.now();
  const child = spawn(process.execPath, [...process.execArgv, script, "program-child", "--addon", options.addon, "--policies", options.policies], { stdio: ["ignore", "pipe", "inherit"] });
  let found: z.infer<typeof programEventSchema> | undefined;
  createInterface({ input: child.stdout }).on("line", line => {
    found = programEventSchema.parse(JSON.parse(line));
  });
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    child.kill("SIGKILL");
  }, options.programCapSeconds * 1000);
  const exited = Promise.withResolvers<void>();
  child.on("close", () => exited.resolve());
  await exited.promise;
  clearTimeout(timer);
  if (found !== undefined) return { status: "ok", compileMs: found.compileMs, warmupMs: found.warmupMs, clauses: found.clauses };
  return { status: timedOut ? "timeout" : "error", wallMs: performance.now() - started };
}

// ---------------------------------------------------------------------------------------------------------------------
// The native oracle cache: per clause, its subset lines and times, and its full-corpus time

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

const timingSchema = z.object({ status: z.enum(["ok", "timeout", "error"]), wallMs: z.number(), cpuMs: z.number().nullable() });
const cacheSchema = z.object({
  format: z.literal("rerail.sql-loop-native-cache/v2"),
  key: z.record(z.string(), z.string()),
  measured: z.string(),
  /** Per clause: native's lines over the subset (the oracle), and its subset and full-corpus times. */
  clauses: z.record(z.string(), z.object({ lines: z.array(z.string()), subset: timingSchema, full: timingSchema })),
  /** Every clause in one workspace (production's shape): native's whole-tree and subset apply. */
  whole: z.object({ full: z.object({ wallMs: z.number(), cpuMs: z.number(), files: z.number().int() }), subset: z.object({ wallMs: z.number(), cpuMs: z.number() }) }),
});
type NativeCache = z.infer<typeof cacheSchema>;

function nativeCacheKey(options: Options, subset: Subset) {
  return {
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

/** The native cache for `key`, measuring what it lacks: each missing clause on the subset and the full corpus, and the whole-program applies. */
async function nativeCache(options: Options, pool: UnitPool, subset: Subset, clauses: readonly Clause[], key: Record<string, string>): Promise<{ cache: NativeCache; measured: string[] }> {
  const path = cachePathOf(key);
  const existing = existsSync(path) ? cacheSchema.parse(JSON.parse(readFileSync(path, "utf8"))) : undefined;
  const missing = clauses.filter(clause => existing?.clauses[clause.name] === undefined).map(clause => clause.name);
  if (existing !== undefined && missing.length === 0) return { cache: existing, measured: [] };
  process.stderr.write(`native cache: measuring ${missing.length} clauses on the subset and the full corpus${existing === undefined ? ", and the whole-program applies" : ""}\n`);
  const sub = await runUnits(pool, { engine: "native", clauses: missing, capSeconds: options.nativeCapSeconds, jobs: options.jobs, label: "native subset" });
  const fullPool = new UnitPool(options, "full");
  const full = await runUnits(fullPool, { engine: "native", clauses: missing, capSeconds: options.nativeCapSeconds, jobs: options.jobs, label: "native full" });
  fullPool.close();
  const timing = (outcome: UnitOutcome | undefined) => outcome === undefined ? { status: "error" as const, wallMs: 0, cpuMs: null } : outcome.status === "ok" ? { status: "ok" as const, wallMs: outcome.wallMs, cpuMs: outcome.cpuMs } : { status: outcome.status, wallMs: outcome.wallMs, cpuMs: null };
  const entries: NativeCache["clauses"] = { ...existing?.clauses };
  for (const clause of missing) {
    const outcome = sub.outcomes.get(clause);
    if (outcome?.status !== "ok") throw new Error(`native did not finish ${clause} on the subset (${outcome?.status ?? "missing"}): the oracle needs it`);
    entries[clause] = { lines: outcome.lines, subset: timing(outcome), full: timing(full.outcomes.get(clause)) };
  }
  let whole = existing?.whole;
  if (whole === undefined) {
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
    whole = { full: { wallMs: fullApply.wallMs, cpuMs: fullApply.cpuMs, files: repository.files.length }, subset: { wallMs: subsetApply.wallMs, cpuMs: subsetApply.cpuMs } };
  }
  const cache: NativeCache = { format: "rerail.sql-loop-native-cache/v2", key, measured: new Date().toISOString(), clauses: entries, whole };
  mkdirSync(resolve(stateRoot, "loop-cache"), { recursive: true });
  writeFileSync(path, `${JSON.stringify(cache)}\n`);
  return { cache, measured: missing };
}

// ---------------------------------------------------------------------------------------------------------------------
// run

const median = (values: readonly number[]) => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length === 0 ? Number.NaN : sorted.length % 2 === 1 ? sorted[middle]! : (sorted[middle - 1]! + sorted[middle]!) / 2;
};
const quantile = (values: readonly number[], q: number) => {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted.length === 0 ? Number.NaN : sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))]!;
};
const spread = (values: readonly number[]) => (values.length < 2 ? 0 : (Math.max(...values) - Math.min(...values)) / Math.min(...values));
const seconds = (ms: number) => `${(ms / 1000).toFixed(2)} s`;
const percent = (value: number) => `${value >= 0 ? "+" : ""}${(value * 100).toFixed(1)}%`;
const sum = (values: Iterable<number>) => {
  let total = 0;
  for (const value of values) total += value;
  return total;
};

const outcomeRecordSchema = z.object({ status: z.enum(["ok", "timeout", "error"]), wallMs: z.number(), cpuMs: z.number().nullable(), warmupMs: z.number().nullable(), files: z.number().int().nullable() });
const runRecordSchema = z.object({
  format: z.literal("rerail.sql-loop-run/v2"),
  started: z.string(),
  addon: z.object({ sha256: z.string() }).passthrough().optional(),
  subsetSha256: z.string(),
  clauseSetSha256: z.string(),
  /** Per round: per clause, SQL's and native's outcome. */
  rounds: z.array(z.object({ sql: z.record(z.string(), outcomeRecordSchema), native: z.record(z.string(), outcomeRecordSchema) })),
  totals: z.object({ sqlCpuMs: z.array(z.number()), sqlWallMs: z.array(z.number()), nativeCpuMs: z.array(z.number()), nativeWallMs: z.array(z.number()) }),
  ratio: z.object({ wall: z.number(), cpu: z.number() }),
}).passthrough();
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

const outcomeRecord = (outcome: UnitOutcome): z.infer<typeof outcomeRecordSchema> => outcome.status === "ok"
  ? { status: "ok", wallMs: outcome.wallMs, cpuMs: outcome.cpuMs, warmupMs: outcome.warmupMs, files: outcome.files }
  : { status: outcome.status, wallMs: outcome.wallMs, cpuMs: null, warmupMs: null, files: null };
/** A clause's time on one engine in a round: CPU where it finished, its wall time where it timed out (a lower bound). */
const unitCpu = (outcome: z.infer<typeof outcomeRecordSchema> | undefined) => outcome === undefined ? 0 : outcome.cpuMs ?? outcome.wallMs;

/**
 * A clause's full-corpus SQL time projected from its subset time: scaled by native's full ÷ subset time for the clause
 * where native spends at least `floor` ms on the subset, else by the pooled native ratio over all clauses.
 */
function projectClause(sqlSubsetMs: number, nativeSubsetMs: number | null, nativeFullMs: number | null, pooled: number): number {
  const floor = 20;
  return nativeSubsetMs !== null && nativeFullMs !== null && nativeSubsetMs >= floor ? sqlSubsetMs * (nativeFullMs / nativeSubsetMs) : sqlSubsetMs * pooled;
}

function pooledNativeScale(cache: NativeCache, clauses: readonly string[]): number {
  const usable = clauses.map(clause => cache.clauses[clause]).filter(entry => entry !== undefined && entry.subset.cpuMs !== null && entry.full.cpuMs !== null);
  return sum(usable.map(entry => entry!.full.cpuMs!)) / Math.max(1, sum(usable.map(entry => entry!.subset.cpuMs!)));
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
  const records = runRecords();
  const previous = records.find(({ record }) => record.subsetSha256 === subsetSha(subset) && record.clauseSetSha256 === clauseSetSha);
  // Costliest first (by the previous run's SQL time; unknown clauses first), so the jobs end together.
  const lastSql = new Map(Object.entries(previous?.record.rounds.at(-1)?.sql ?? {}).map(([clause, outcome]) => [clause, unitCpu(outcome)]));
  const ordered = [...names].sort((a, b) => (lastSql.get(b) ?? Infinity) - (lastSql.get(a) ?? Infinity) || compareStrings(a, b));
  console.log(`sql-loop: ${names.length} clauses alone over ${subset.files.length} subset files; ${options.rounds} round(s), ${options.jobs} jobs, ${options.capSeconds} s cap per clause; background load ${backgroundLoad.map(value => value.toFixed(1)).join(" ")}`);

  const key = nativeCacheKey(options, subset);
  const pool = new UnitPool(options, "subset");
  const { cache, measured } = await nativeCache(options, pool, subset, clauses, key);
  if (measured.length > 0) console.log(`  native cache: measured ${measured.length} clauses (subset and full corpus)`);
  const rounds: { sql: Map<string, UnitOutcome>; native: Map<string, UnitOutcome>; sqlWallMs: number; nativeWallMs: number }[] = [];
  const timedOut = new Set<string>();
  for (let round = 1; round <= options.rounds; round += 1) {
    const native = options.native === "fresh"
      ? await runUnits(pool, { engine: "native", clauses: ordered, capSeconds: options.nativeCapSeconds, jobs: options.jobs, label: `native ${round}` })
      : undefined;
    // A clause that timed out stays out of later rounds: its TIMEOUT row stands.
    const sql = await runUnits(pool, { engine: "sql", clauses: ordered.filter(clause => !timedOut.has(clause)), capSeconds: options.capSeconds, jobs: options.jobs, label: `sql ${round}` });
    for (const [clause, outcome] of sql.outcomes) if (outcome.status !== "ok") timedOut.add(clause);
    rounds.push({ sql: sql.outcomes, native: native?.outcomes ?? new Map(), sqlWallMs: sql.wallMs, nativeWallMs: native?.wallMs ?? 0 });
    console.error(`  round ${round}: native ${native === undefined ? "cached" : seconds(native.wallMs)}, SQL ${seconds(sql.wallMs)} (wall, ${options.jobs} jobs)`);
  }
  pool.close();
  const ready = pool.ready;
  const program = options.programCapSeconds > 0 ? await runProgram(options) : undefined;

  const problems: string[] = [];
  // Parity: the last round with each clause's SQL lines (a TIMEOUT has none).
  console.log("");
  const lastOk = new Map<string, Extract<UnitOutcome, { status: "ok" }>>();
  for (const round of rounds) for (const [clause, outcome] of round.sql) if (outcome.status === "ok") lastOk.set(clause, outcome);
  for (const round of rounds) {
    for (const [clause, outcome] of round.native) {
      if (outcome.status !== "ok") problems.push(`native ${outcome.status} on ${clause}`);
      else if (sha256(outcome.lines.join("\n")) !== sha256(cache.clauses[clause]!.lines.join("\n"))) problems.push(`native's lines for ${clause} differ from the cached oracle`);
    }
  }
  for (const round of rounds.slice(1)) {
    for (const [clause, outcome] of round.sql) {
      const first = rounds[0]!.sql.get(clause);
      if (outcome.status === "ok" && first?.status === "ok" && outcome.lines.join("\n") !== first.lines.join("\n")) problems.push(`SQL rounds disagree on ${clause}`);
    }
  }
  const parities = [...lastOk].map(([clause, outcome]) => compareLines(clause, cache.clauses[clause]!.lines, outcome.lines));
  const differing = parities.filter(parity => parity.files.length > 0);
  const timeouts = names.filter(clause => !lastOk.has(clause));
  const refused = [...lastOk.values()].filter(outcome => outcome.refused.length > 0);
  console.log(`PARITY ${differing.length === 0 ? "OK" : "DIFFERS"}: ${parities.length} clauses compared (${sum(parities.map(parity => parity.compared))} native lines), ${differing.length} differ; ${timeouts.length} not compared (TIMEOUT/ERROR); ${sum(parities.map(parity => parity.budgetExcluded.length))} (clause, file)s excluded for native budget failures; SQL refused ${refused.length} clauses`);
  for (const parity of differing.slice(0, options.show)) {
    console.log(`  ${parity.clause}: ${parity.files.length} files differ`);
    for (const file of parity.files.slice(0, 3)) {
      console.log(`    @ ${file.path || "(no file)"}: ${file.onlyNative.length} only native, ${file.onlySql.length} only SQL`);
      for (const line of file.onlyNative.slice(0, 2)) console.log(`      - native: ${line.slice(0, 240)}`);
      for (const line of file.onlySql.slice(0, 2)) console.log(`      + sql:    ${line.slice(0, 240)}`);
    }
  }
  for (const outcome of refused.slice(0, options.show)) console.log(`  refused ${outcome.clause}: ${outcome.refused[0]}`);

  // Speed: per round, summed over clauses (a TIMEOUT counts its time at the cap: a lower bound).
  const total = (outcomes: Map<string, UnitOutcome>, field: "cpuMs" | "wallMs") => sum([...outcomes.values()].map(outcome => outcome.status === "ok" ? outcome[field] : outcome.wallMs));
  const nativeCpu = options.native === "fresh" ? rounds.map(round => total(round.native, "cpuMs")) : [sum(names.map(clause => cache.clauses[clause]!.subset.cpuMs ?? 0))];
  const nativeWall = options.native === "fresh" ? rounds.map(round => total(round.native, "wallMs")) : [sum(names.map(clause => cache.clauses[clause]!.subset.wallMs))];
  // Clauses every round ran, so rounds compare like for like.
  const common = names.filter(clause => rounds.every(round => round.sql.get(clause)?.status === "ok"));
  const commonTotal = (round: (typeof rounds)[number], field: "cpuMs" | "wallMs") => sum(common.map(clause => { const outcome = round.sql.get(clause)!; return outcome.status === "ok" ? outcome[field] : 0; }));
  const sqlCpu = rounds.map(round => total(round.sql, "cpuMs") + sum([...timedOut].filter(clause => !round.sql.has(clause)).map(clause => rounds[0]!.sql.get(clause)?.wallMs ?? 0)));
  const sqlWall = rounds.map(round => total(round.sql, "wallMs") + sum([...timedOut].filter(clause => !round.sql.has(clause)).map(clause => rounds[0]!.sql.get(clause)?.wallMs ?? 0)));
  const ratio = { wall: median(sqlWall) / median(nativeWall), cpu: median(sqlCpu) / median(nativeCpu) };
  const untrusted: string[] = [];
  if (backgroundLoad[0]! > LOAD_LIMIT) untrusted.push(`background load ${backgroundLoad[0]!.toFixed(1)} > ${LOAD_LIMIT}`);
  const sqlSpread = spread(rounds.map(round => commonTotal(round, "cpuMs")));
  if (sqlSpread > SPREAD_LIMIT) untrusted.push(`SQL rounds spread ${percent(sqlSpread)} in CPU`);
  if (options.native === "fresh" && spread(nativeCpu) > SPREAD_LIMIT) untrusted.push(`native rounds spread ${percent(spread(nativeCpu))} in CPU`);
  if (rounds.length < 2) untrusted.push("1 round: spread unknown");
  if (timeouts.length > 0) untrusted.push(`${timeouts.length} TIMEOUT clauses counted at their cap`);
  const unitLoads = rounds.flatMap(round => [...round.sql.values()].flatMap(outcome => outcome.status === "ok" ? [outcome.load] : []));
  console.log("");
  console.log(`SPEED${untrusted.length > 0 ? ` (UNTRUSTED: ${untrusted.join("; ")})` : " (trusted)"}`);
  for (const [index, round] of rounds.entries()) {
    console.log(`  round ${index + 1}: SQL Σ ${seconds(sqlCpu[index]!)} CPU, Σ ${seconds(sqlWall[index]!)} wall (phase ${seconds(round.sqlWallMs)} over ${options.jobs} jobs); native ${options.native === "fresh" ? `Σ ${seconds(nativeCpu[index]!)} CPU, Σ ${seconds(nativeWall[index]!)} wall (phase ${seconds(round.nativeWallMs)})` : "cached"}`);
  }
  console.log(`  SQL/native over ${names.length} clauses: CPU ${ratio.cpu.toFixed(2)}×, wall ${ratio.wall.toFixed(2)}× (medians of the rounds' sums; native ${options.native === "fresh" ? "fresh, alternating" : `cached ${cache.measured}`}); load during units median ${median(unitLoads).toFixed(1)}, max ${Math.max(0, ...unitLoads).toFixed(1)}`);
  const warmups = [...lastOk.values()].map(outcome => outcome.warmupMs);
  console.log(`  per-clause SQL setup (compile + every worker's session, untimed): Σ ${seconds(sum(warmups))}, max ${seconds(Math.max(0, ...warmups))}`);
  if (timeouts.length > 0) {
    console.log("  TIMEOUT:");
    for (const clause of timeouts) {
      const outcome = rounds.map(round => round.sql.get(clause)).find(found => found !== undefined && found.status !== "ok");
      if (outcome !== undefined) console.log(`    ${outcome.status.toUpperCase().padEnd(7)} ${seconds(outcome.wallMs).padStart(9)}  ${clause}: ${outcome.detail}`);
    }
  }
  // Statements, clauses, (clause, file)s: the last round each clause finished in.
  const statements = new Map<string, { milliseconds: number; rows: number; runs: number; clauses: Set<string> }>();
  for (const [clause, outcome] of lastOk) {
    for (const statement of outcome.statements) {
      const found = statements.get(statement.sql) ?? { milliseconds: 0, rows: 0, runs: 0, clauses: new Set<string>() };
      statements.set(statement.sql, { milliseconds: found.milliseconds + statement.milliseconds, rows: found.rows + statement.rows, runs: found.runs + statement.runs, clauses: found.clauses.add(clause) });
    }
  }
  const topStatements = [...statements].map(([sql, entry]) => ({ sql, milliseconds: entry.milliseconds, rows: entry.rows, runs: entry.runs, clauses: [...entry.clauses].slice(0, 5) })).sort((a, b) => b.milliseconds - a.milliseconds);
  const clauseTimes = names.map(clause => ({ clause, milliseconds: median(rounds.flatMap(round => { const outcome = round.sql.get(clause); return outcome === undefined ? [] : [outcome.status === "ok" ? outcome.cpuMs : outcome.wallMs]; })), status: lastOk.has(clause) ? "ok" : "timeout", native: cache.clauses[clause]!.subset.cpuMs ?? 0 }))
    .sort((a, b) => b.milliseconds - a.milliseconds);
  const clauseFiles = [...lastOk].flatMap(([clause, outcome]) => outcome.clauseFiles.map(entry => ({ clause, ...entry }))).sort((a, b) => b.milliseconds - a.milliseconds);
  console.log("  top statements by SQL time (Σ over clauses and workers; last round each clause finished):");
  for (const statement of topStatements.slice(0, 10)) console.log(`    ${seconds(statement.milliseconds).padStart(9)} ${String(statement.rows).padStart(10)} rows ${String(statement.runs).padStart(6)} runs  [${statement.clauses.slice(0, 2).join(", ")}] ${statement.sql.replace(/\s+/g, " ").slice(0, 150)}`);
  console.log("  top clauses by SQL CPU (median over rounds; native subset CPU):");
  for (const entry of clauseTimes.slice(0, options.show)) console.log(`    ${seconds(entry.milliseconds).padStart(9)} ${entry.status === "ok" ? "" : "TIMEOUT "}(native ${seconds(entry.native)})  ${entry.clause}`);
  console.log("  top (clause, file) by SQL time (a batch's time split evenly over its files):");
  for (const entry of clauseFiles.slice(0, 10)) console.log(`    ${seconds(entry.milliseconds).padStart(9)}  ${entry.clause} @ ${entry.path}`);

  // Against the previous run on this subset and clause set
  if (previous !== undefined) {
    const before = previous.record;
    console.log("");
    console.log(`AGAINST ${previous.path.replace(homedir(), "~")} (${before.started}):`);
    console.log(`  SQL Σ CPU ${seconds(median(before.totals.sqlCpuMs))} → ${seconds(median(sqlCpu))} (${percent(median(sqlCpu) / median(before.totals.sqlCpuMs) - 1)}), Σ wall ${seconds(median(before.totals.sqlWallMs))} → ${seconds(median(sqlWall))}; ratio CPU ${before.ratio.cpu.toFixed(2)}× → ${ratio.cpu.toFixed(2)}×, wall ${before.ratio.wall.toFixed(2)}× → ${ratio.wall.toFixed(2)}×`);
    const old = new Map(names.map(clause => [clause, median(before.rounds.map(round => unitCpu(round.sql[clause])))]));
    const deltas = clauseTimes.map(entry => ({ ...entry, before: old.get(entry.clause) ?? 0 })).map(entry => ({ ...entry, delta: entry.milliseconds - entry.before })).sort((a, b) => Math.abs(b.delta) - Math.abs(a.delta));
    console.log("  largest clause changes:");
    for (const entry of deltas.slice(0, 8)) console.log(`    ${entry.delta >= 0 ? "+" : ""}${seconds(entry.delta).padStart(9)}  ${entry.clause} (${seconds(entry.before)} → ${seconds(entry.milliseconds)})`);
    const newTimeouts = timeouts.filter(clause => before.rounds.every(round => round.sql[clause]?.status === "ok"));
    if (newTimeouts.length > 0) console.log(`  new TIMEOUTs: ${newTimeouts.join(", ")}`);
  }

  // Projection: each clause's subset SQL time scaled to the full corpus by native's own scale for the clause.
  const pooled = pooledNativeScale(cache, names);
  const projected = clauseTimes.map(entry => {
    const native = cache.clauses[entry.clause]!;
    return { clause: entry.clause, sqlFullMs: projectClause(entry.milliseconds, native.subset.cpuMs, native.full.cpuMs, pooled), nativeFullMs: native.full.cpuMs ?? native.full.wallMs, timeout: entry.status !== "ok" };
  });
  const correction = subset.calibration?.correction ?? 1;
  const projectedSql = sum(projected.map(entry => entry.sqlFullMs)) * correction;
  const nativeFull = sum(projected.map(entry => entry.nativeFullMs));
  const projectedRatio = projectedSql / nativeFull;
  console.log("");
  console.log(`PROJECTION (full corpus, clauses alone): Σ SQL ≈ ${seconds(projectedSql)} CPU vs native Σ ${seconds(nativeFull)} → ${projectedRatio.toFixed(1)}×${subset.calibration === null ? " — UNCALIBRATED" : ` (correction ${correction.toFixed(2)}, error ±${(subset.calibration.error * 100).toFixed(0)}%, calibrated ${subset.calibration.date})`}${timeouts.length > 0 ? `; ${timeouts.length} TIMEOUT clauses projected from their cap (lower bound)` : ""}`);
  console.log(`  composed as native's whole tree (${seconds(cache.whole.full.wallMs)} wall, ${seconds(cache.whole.full.cpuMs)} CPU): SQL ≈ ${seconds(cache.whole.full.wallMs * projectedRatio)} wall; target ≤ 1.10× = ${seconds(cache.whole.full.wallMs * 1.1)}; whole-program setup not included (PROGRAM)`);
  for (const entry of [...projected].sort((a, b) => b.sqlFullMs - a.sqlFullMs).slice(0, 5)) console.log(`    ${seconds(entry.sqlFullMs * correction).padStart(10)}${entry.timeout ? "+" : " "} (native ${seconds(entry.nativeFullMs)})  ${entry.clause}`);
  if (program !== undefined) {
    console.log("");
    console.log(program.status === "ok"
      ? `PROGRAM: ${program.clauses} clauses in one SQL program: rules compiled in ${seconds(program.compileMs)}, program compile + first session ${seconds(program.warmupMs)}`
      : `PROGRAM ${program.status.toUpperCase()}: compiling every clause into one SQL program and opening a first session ${program.status === "timeout" ? `passed the ${options.programCapSeconds} s cap` : "failed"} (${seconds(program.wallMs)}); the whole-tree run pays this before any file`);
  }

  // Record
  const record = {
    format: "rerail.sql-loop-run/v2" as const,
    started: startedAt.toISOString(),
    turnaroundMs: performance.now() - started,
    machine: { cpus: cpus().length, model: cpus()[0]?.model, memoryBytes: totalmem(), platform: `${process.platform} ${process.arch}`, node: process.version },
    git: { head: git(rerailRoot, "rev-parse", "HEAD").trim(), dirty: git(rerailRoot, "status", "--porcelain", "--untracked-files=no", "--", "native", "Cargo.toml").split("\n").filter(line => line.length > 0) },
    addon: ready?.addon ?? engine.addon,
    subsetSha256: subsetSha(subset),
    subsetFiles: subset.files.length,
    clauseSetSha256: clauseSetSha,
    clauseCount: names.length,
    nativeCache: { path: cachePathOf(key), key, measured: cache.measured, measuredNow: measured.length },
    options: { rounds: options.rounds, native: options.native, jobs: options.jobs, capSeconds: options.capSeconds, clauses: options.clauses, programCapSeconds: options.programCapSeconds },
    load: { background: backgroundLoad, unitsMedian: median(unitLoads), unitsMax: Math.max(0, ...unitLoads) },
    rounds: rounds.map(round => ({
      sqlPhaseWallMs: round.sqlWallMs,
      nativePhaseWallMs: round.nativeWallMs,
      sql: Object.fromEntries([...round.sql].map(([clause, outcome]) => [clause, outcomeRecord(outcome)])),
      native: Object.fromEntries([...round.native].map(([clause, outcome]) => [clause, outcomeRecord(outcome)])),
    })),
    totals: { sqlCpuMs: sqlCpu, sqlWallMs: sqlWall, nativeCpuMs: nativeCpu, nativeWallMs: nativeWall },
    ratio,
    parity: { equal: differing.length === 0, compared: parities.length, differing, timeouts, refused: refused.map(outcome => ({ clause: outcome.clause, reasons: outcome.refused })) },
    statements: topStatements.slice(0, 50),
    clauses: clauseTimes,
    clauseFiles: clauseFiles.slice(0, 200),
    projection: { sqlFullMs: projectedSql, nativeFullMs: nativeFull, ratio: projectedRatio, correction, error: subset.calibration?.error ?? null, wholeTreeWallMs: cache.whole.full.wallMs * projectedRatio, clauses: projected },
    program: program ?? null,
    trusted: untrusted.length === 0,
    untrusted,
    problems,
  };
  mkdirSync(resolve(stateRoot, "loop-runs"), { recursive: true });
  const recordPath = resolve(stateRoot, "loop-runs", `${stamp}.json`);
  writeFileSync(recordPath, `${JSON.stringify(record)}\n`);
  console.log("");
  for (const problem of problems) console.log(`PROBLEM: ${problem}`);
  const verdict = problems.length > 0 ? "ERROR" : differing.length > 0 ? "FAIL" : "PASS";
  console.log(`VERDICT ${verdict}: parity ${differing.length === 0 ? "ok" : `${differing.length} clauses differ`}${timeouts.length > 0 ? `, ${timeouts.length} TIMEOUT` : ""}; SQL/native CPU ${ratio.cpu.toFixed(2)}×, wall ${ratio.wall.toFixed(2)}×${untrusted.length > 0 ? " (untrusted timing)" : ""}; projected full corpus ${projectedRatio.toFixed(1)}×; ${seconds(performance.now() - started)} for the run; record ${recordPath.replace(homedir(), "~")}`);
  return verdict === "PASS" ? 0 : verdict === "FAIL" ? 1 : 3;
}

// ---------------------------------------------------------------------------------------------------------------------
// calibrate: SQL per clause on the full corpus, against the subset run's projection

const calibrationDataSchema = z.object({
  format: z.literal("rerail.sql-loop-calibration/v1"),
  addon: z.string(),
  measured: z.string(),
  capSeconds: z.number(),
  /** Per clause: SQL alone over the full corpus. */
  clauses: z.record(z.string(), timingSchema),
});

async function calibrate(options: Options): Promise<number> {
  const subset = readSubset(options.subset);
  const engine = loadEngine(options.addon);
  const clauses = reportingClauses(loadRules(engine, options.policies), options.clauses);
  const names = clauses.map(clause => clause.name);
  const key = nativeCacheKey(options, subset);
  const subsetPool = new UnitPool(options, "subset");
  const { cache } = await nativeCache(options, subsetPool, subset, reportingClauses(loadRules(engine, options.policies)), key);
  subsetPool.close();
  // The subset's SQL times: the latest run of this addon on this subset that covers the clauses.
  const latest = runRecords().find(({ record }) => record.addon?.sha256 === engine.addon.sha256 && record.subsetSha256 === subsetSha(subset) && names.every(clause => record.rounds.some(round => round.sql[clause] !== undefined)));
  if (latest === undefined) throw new UsageError(`no loop run of ${engine.addon.sha256} on this subset covers these clauses: \`loop.ts run --addon ${options.addon}\` first`);
  const dataPath = resolve(stateRoot, "loop-calibration", `${engine.addon.sha256.slice("sha256:".length, "sha256:".length + 16)}.json`);
  const data = existsSync(dataPath) ? calibrationDataSchema.parse(JSON.parse(readFileSync(dataPath, "utf8"))) : { format: "rerail.sql-loop-calibration/v1" as const, addon: engine.addon.sha256, measured: "", capSeconds: options.capSeconds, clauses: {} };
  const missing = names.filter(clause => data.clauses[clause] === undefined || (data.clauses[clause]!.status !== "ok" && data.capSeconds < options.capSeconds));
  // Native's full-corpus time orders the work: costliest first.
  missing.sort((a, b) => (cache.clauses[b]?.full.cpuMs ?? 0) - (cache.clauses[a]?.full.cpuMs ?? 0) || compareStrings(a, b));
  console.log(`calibrate: ${names.length} clauses; ${missing.length} to run on SQL over the full corpus (${options.jobs} jobs, ${options.capSeconds} s cap); subset run ${latest.path.replace(homedir(), "~")}; load ${loadavg()[0]!.toFixed(1)}`);
  if (missing.length > 0) {
    const pool = new UnitPool(options, "full");
    const full = await runUnits(pool, { engine: "sql", clauses: missing, capSeconds: options.capSeconds, jobs: options.jobs, label: "sql full" });
    pool.close();
    for (const clause of missing) {
      const outcome = full.outcomes.get(clause);
      data.clauses[clause] = outcome === undefined ? { status: "error", wallMs: 0, cpuMs: null } : outcome.status === "ok" ? { status: "ok", wallMs: outcome.wallMs, cpuMs: outcome.cpuMs } : { status: outcome.status, wallMs: outcome.wallMs, cpuMs: null };
    }
    data.measured = new Date().toISOString();
    data.capSeconds = Math.max(data.capSeconds, options.capSeconds);
    mkdirSync(resolve(stateRoot, "loop-calibration"), { recursive: true });
    writeFileSync(dataPath, `${JSON.stringify(data, null, 2)}\n`);
  }
  // Every clause the data holds, projected from the subset run.
  const all = reportingClauses(loadRules(engine, options.policies)).map(clause => clause.name);
  const pooled = pooledNativeScale(cache, all);
  const rows = all.flatMap(clause => {
    const actual = data.clauses[clause];
    const sqlSubset = median(latest.record.rounds.flatMap(round => round.sql[clause]?.status === "ok" ? [round.sql[clause]!.cpuMs!] : []));
    const native = cache.clauses[clause]!;
    if (actual === undefined || Number.isNaN(sqlSubset)) return [];
    return [{
      clause, policy: clause.slice(0, clause.indexOf("/")), status: actual.status, actualMs: actual.cpuMs ?? actual.wallMs, sqlSubsetMs: sqlSubset,
      nativeSubsetMs: native.subset.cpuMs, nativeFullMs: native.full.cpuMs,
      projectedMs: projectClause(sqlSubset, native.subset.cpuMs, native.full.cpuMs, pooled),
    }];
  });
  const calibrated = rows.filter(row => row.status === "ok");
  const totalActual = sum(calibrated.map(row => row.actualMs));
  const totalProjected = sum(calibrated.map(row => row.projectedMs));
  const correction = totalActual / totalProjected;
  const relative = calibrated.filter(row => row.actualMs >= 100).map(row => Math.abs(row.projectedMs / row.actualMs - 1));
  // Leave each policy out of the correction that projects it.
  const policies = [...new Set(calibrated.map(row => row.policy))];
  const errorParts = policies.map(policy => {
    const others = calibrated.filter(row => row.policy !== policy);
    const factor = others.length === 0 ? 1 : sum(others.map(row => row.actualMs)) / sum(others.map(row => row.projectedMs));
    const own = calibrated.filter(row => row.policy === policy);
    return { policy, actual: sum(own.map(row => row.actualMs)), corrected: sum(own.map(row => row.projectedMs)) * factor };
  });
  const error = sum(errorParts.map(part => Math.abs(part.corrected - part.actual))) / totalActual;
  const calibration: Calibration = {
    date: new Date().toISOString(),
    addon: engine.addon.sha256,
    head: git(rerailRoot, "rev-parse", "HEAD").trim(),
    correction,
    error,
    totalError: totalProjected / totalActual - 1,
    medianAbsError: median(relative),
    p90AbsError: quantile(relative, 0.9),
    clauses: calibrated.length,
    timeouts: rows.filter(row => row.status !== "ok").map(row => row.clause),
    note: `SQL alone per clause over the full corpus (cap ${data.capSeconds} s) against the subset run ${latest.record.started} projected by native's per-clause full ÷ subset CPU; error: Σ|corrected − actual| per policy ÷ Σ actual, each policy's correction from the others`,
    policies: errorParts.map(part => ({ policy: part.policy, actualMs: part.actual, correctedMs: part.corrected })),
  };
  const manifest = readSubset(options.subset);
  writeFileSync(options.subset, `${JSON.stringify({ ...manifest, calibration }, null, 2)}\n`);
  console.log(`calibration over ${calibrated.length} clauses (${calibration.timeouts.length} past the cap): Σ actual ${seconds(totalActual)}, Σ projected ${seconds(totalProjected)} (${percent(calibration.totalError)}); correction ${correction.toFixed(3)}; error ±${(error * 100).toFixed(1)}% (leave one policy out); per clause ≥ 0.1 s: median |error| ${(calibration.medianAbsError * 100).toFixed(0)}%, p90 ${(calibration.p90AbsError * 100).toFixed(0)}%`);
  console.log("  policy                              actual     corrected");
  for (const part of errorParts.sort((a, b) => b.actual - a.actual)) console.log(`  ${part.policy.padEnd(34)} ${seconds(part.actual).padStart(10)} ${seconds(part.corrected).padStart(12)}`);
  console.log("  costliest clauses: actual, projected, SQL subset, native subset → full");
  for (const row of [...rows].sort((a, b) => b.actualMs - a.actualMs).slice(0, options.show)) {
    console.log(`  ${seconds(row.actualMs).padStart(10)}${row.status === "ok" ? " " : "+"} ${seconds(row.projectedMs).padStart(10)} ${seconds(row.sqlSubsetMs).padStart(9)}  ${row.nativeSubsetMs === null ? "?" : seconds(row.nativeSubsetMs)} → ${row.nativeFullMs === null ? "?" : seconds(row.nativeFullMs)}  ${row.clause}`);
  }
  if (options.output !== undefined) writeFileSync(options.output, `${JSON.stringify({ calibration, rows }, null, 2)}\n`);
  console.log(`  wrote the calibration into ${options.subset}; data ${dataPath.replace(homedir(), "~")}`);
  return 0;
}

// ---------------------------------------------------------------------------------------------------------------------
// costs: each (clause, file)'s SQL time, one file per apply, each capped

const costsFileSchema = z.object({
  format: z.literal("rerail.sql-loop-costs/v2"),
  addon: z.string(),
  measured: z.string(),
  capSeconds: z.number(),
  /** Every measured (clause, file): its single-file SQL apply wall time; a capped one holds the cap. */
  pairs: z.array(z.object({ clause: z.string(), path: z.string(), milliseconds: z.number(), capped: z.boolean() })),
  /** Clauses measured to the end, or stopped. */
  done: z.array(z.string()),
  /** Clauses stopped after `--max-capped` capped files: their other files are unmeasured. */
  stopped: z.array(z.object({ clause: z.string(), detail: z.string() })),
});
type CostsFile = z.infer<typeof costsFileSchema>;

/**
 * Times each (clause, file) of the subset (or `--files-from`) on SQL, one file per apply, each capped; a clause stops
 * after `--max-capped` capped files. Resumes: the clauses the costs file already finished for this addon are skipped.
 * The file is rewritten as clauses finish, so a run cut short keeps what it measured.
 */
async function costs(options: Options): Promise<number> {
  const started = performance.now();
  const engine = loadEngine(options.addon);
  const output = options.output ?? options.costs;
  const existing = existsSync(output) ? costsFileSchema.parse(JSON.parse(readFileSync(output, "utf8"))) : undefined;
  const reuse = existing?.addon === engine.addon.sha256 && existing.capSeconds === options.capSeconds;
  const merged = new Map((reuse ? existing.pairs : []).map(pair => [`${pair.clause}\t${pair.path}`, pair]));
  const finished = new Set(reuse ? existing.done : []);
  const stopped = new Map((reuse ? existing.stopped : []).map(entry => [entry.clause, entry.detail]));
  const names = reportingClauses(loadRules(engine, options.policies), options.clauses).map(clause => clause.name).filter(clause => !finished.has(clause));
  console.log(`costs: ${names.length} clauses to measure (${finished.size} already measured for this addon), ${options.jobs} jobs, ${options.capSeconds} s cap per (clause, file), ${options.maxCapped} capped files per clause`);
  let flushed = 0;
  const flush = (pairs: readonly PairCost[], outcomes: ReadonlyMap<string, UnitOutcome>) => {
    for (const pair of pairs) merged.set(`${pair.clause}\t${pair.path}`, pair);
    for (const [clause, outcome] of outcomes) {
      finished.add(clause);
      if (outcome.status !== "ok") stopped.set(clause, outcome.detail);
    }
    const file: CostsFile = {
      format: "rerail.sql-loop-costs/v2", addon: engine.addon.sha256, measured: new Date().toISOString(), capSeconds: options.capSeconds,
      pairs: [...merged.values()].sort((a, b) => b.milliseconds - a.milliseconds),
      done: [...finished].sort(compareStrings),
      stopped: [...stopped].map(([clause, detail]) => ({ clause, detail })).sort((a, b) => compareStrings(a.clause, b.clause)),
    };
    writeFileSync(output, `${JSON.stringify(file)}\n`);
  };
  const pool = new UnitPool(options, options.filesFrom === undefined ? "subset" : "list", options.filesFrom);
  const result = await runUnits(pool, {
    engine: "sql", clauses: names, capSeconds: options.capSeconds, jobs: options.jobs, perFile: true, maxCappedFiles: options.maxCapped, label: "costs",
    onUnit: (pairs, outcomes) => {
      flushed += 1;
      if (flushed % 20 === 0) flush(pairs, outcomes);
    },
  });
  pool.close();
  flush(result.pairs, result.outcomes);
  const byFile = new Map<string, number>();
  for (const pair of merged.values()) byFile.set(pair.path, (byFile.get(pair.path) ?? 0) + pair.milliseconds);
  const all = [...merged.values()];
  console.log(`costs: ${result.pairs.length} (clause, file)s measured now in ${seconds(performance.now() - started)}; in the file: ${all.length} (clause, file)s, Σ ${seconds(sum(all.map(pair => pair.milliseconds)))}, ${all.filter(pair => pair.capped).length} capped at ${options.capSeconds} s; ${stopped.size} clauses stopped`);
  for (const pair of all.slice(0, 15)) console.log(`  ${(pair.capped ? `>${options.capSeconds} s` : seconds(pair.milliseconds)).padStart(9)}  ${pair.clause} @ ${pair.path}`);
  console.log("  costliest files (Σ over clauses):");
  for (const [path, milliseconds] of [...byFile].sort((a, b) => b[1] - a[1]).slice(0, 10)) console.log(`  ${seconds(milliseconds).padStart(9)}  ${path}`);
  for (const [clause, detail] of stopped) console.log(`  stopped ${clause}: ${detail}`);
  console.log(`  wrote ${output}`);
  return 0;
}

// ---------------------------------------------------------------------------------------------------------------------
// select

/** A deterministic PRNG (mulberry32) for the fixed-seed sample. */
function random(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

const sizeBuckets = (bytes: readonly number[]) => {
  const sorted = [...bytes].sort((a, b) => a - b);
  return [0.25, 0.5, 0.75].map(q => sorted[Math.floor(q * (sorted.length - 1))]!);
};

/** A file's stratum for the sample: its top directory and its size quartile. */
function stratum(path: string, bytes: number, buckets: readonly number[]): string {
  const top = path.includes("/") ? path.slice(0, path.indexOf("/")) : "(root)";
  return `${top}|q${buckets.filter(bound => bytes > bound).length + 1}`;
}

/**
 * The subset: (a) per clause, a file native reports it on and a clean file in its scope, cheapest first; (c) a
 * fixed-seed sample stratified by top directory and size quartile, a file over `--file-limit` giving way to the next of
 * its stratum; (b) per hot clause (one with a (clause, file) over the limit), at most one such file, the cheapest,
 * while the subset's summed SQL stays within `--sql-budget`. SQL costs come from `loop.ts costs` (per (clause, file));
 * a file never measured is estimated from its size.
 */
async function select(options: Options): Promise<number> {
  const engine = loadEngine(options.addon);
  const rules = loadRules(engine, options.policies);
  const repository = collectRepository(engine, options.corpus);
  const bytes = new Map(repository.files.map(file => [file.path, Buffer.byteLength(file.source)]));
  const byPath = new Map(repository.files.map(file => [file.path, file]));
  console.log(`select: ${repository.files.length} corpus files, ${rules.length} rules; native over the full tree…`);
  const timedNative = async (files: readonly SourceFile[]) => {
    const workspace = new RuleWorkspace(engine, rules, { engine: "native" });
    try {
      return await workspace.apply(files.filter(file => workspace.accepts(file.path)));
    } finally {
      workspace.close();
    }
  };
  const full = await timedNative(repository.files);
  const linterOf = new Map(rules.map(rule => [documentSchema.parse(rule.document).id, rule.linter]));
  const findings = new Map<string, Map<string, number>>();
  for (const violation of full.result.violations) {
    const name = `${linterOf.get(violation.ruleId) ?? violation.ruleId}/${violation.clauseId}`;
    const paths = findings.get(name) ?? new Map<string, number>();
    findings.set(name, paths);
    paths.set(violation.path ?? "", (paths.get(violation.path ?? "") ?? 0) + 1);
  }
  const accepted = new Map<string, string[]>();
  for (const rule of rules) {
    const workspace = new RuleWorkspace(engine, [rule], { engine: "native" });
    accepted.set(rule.linter, repository.files.filter(file => workspace.accepts(file.path)).map(file => file.path));
    workspace.close();
  }
  const costed = existsSync(options.costs) ? costsFileSchema.parse(JSON.parse(readFileSync(options.costs, "utf8"))) : undefined;
  const pairs = costed?.pairs ?? [];
  const measured = new Map<string, number>();
  for (const pair of pairs) measured.set(pair.path, (measured.get(pair.path) ?? 0) + pair.milliseconds);
  const perByte = median([...measured].map(([path, milliseconds]) => milliseconds / Math.max(1, bytes.get(path) ?? 1)));
  const sqlCost = (path: string) => measured.get(path) ?? (Number.isNaN(perByte) ? 0 : perByte * bytes.get(path)!);
  const limitMs = options.fileLimitSeconds * 1000;
  const budgetMs = options.sqlBudgetSeconds * 1000;
  const expensive = (path: string) => sqlCost(path) > limitMs;
  const excluded = new Map<string, string>();
  const cheapest = (paths: Iterable<string>) => {
    const sorted = [...paths].sort((a, b) => sqlCost(a) - sqlCost(b) || bytes.get(a)! - bytes.get(b)! || compareStrings(a, b));
    for (const path of sorted.filter(expensive)) excluded.set(path, `SQL ${seconds(sqlCost(path))} over the ${options.fileLimitSeconds} s file limit`);
    return sorted;
  };
  const reasons = new Map<string, Set<string>>();
  const add = (path: string, reason: string) => {
    const found = reasons.get(path) ?? new Set<string>();
    reasons.set(path, found);
    found.add(reason);
  };
  const clauses = reportingClauses(rules);
  // (a) coverage: the most constrained clauses first, each satisfied by a cheap file already chosen where one is.
  const findingFile = new Map<string, string>();
  for (const clause of [...clauses].sort((a, b) => (findings.get(a.name)?.size ?? 0) - (findings.get(b.name)?.size ?? 0) || compareStrings(a.name, b.name))) {
    const candidates = [...(findings.get(clause.name)?.keys() ?? [])].filter(path => byPath.has(path));
    if (candidates.length === 0) continue;
    const chosen = candidates.find(path => reasons.has(path) && !expensive(path)) ?? cheapest(candidates)[0]!;
    findingFile.set(clause.name, chosen);
    add(chosen, "coverage:finding");
  }
  const cleanFile = new Map<string, string>();
  for (const clause of clauses) {
    const found = findings.get(clause.name);
    const candidates = accepted.get(clause.linter)!.filter(path => !found?.has(path));
    if (candidates.length === 0) continue;
    const chosen = cheapest(candidates.filter(path => reasons.has(path)))[0] ?? cheapest(candidates)[0]!;
    cleanFile.set(clause.name, chosen);
    add(chosen, "coverage:clean");
  }
  // (c) representativeness: a stratified systematic sample, fixed seed.
  if (options.sample > 0) {
    const next = random(options.seed);
    const buckets = sizeBuckets([...bytes.values()]);
    const keyed = repository.files.map(file => ({ path: file.path, stratum: stratum(file.path, bytes.get(file.path)!, buckets), order: next() }));
    keyed.sort((a, b) => compareStrings(a.stratum, b.stratum) || a.order - b.order);
    const step = keyed.length / options.sample;
    const start = next() * step;
    for (let index = 0; index < options.sample; index += 1) {
      let at = Math.floor(start + index * step);
      while (expensive(keyed[at]!.path) && at + 1 < keyed.length && keyed[at + 1]!.stratum === keyed[at]!.stratum) {
        excluded.set(keyed[at]!.path, `sampled, but SQL ${seconds(sqlCost(keyed[at]!.path))} is over the ${options.fileLimitSeconds} s file limit`);
        at += 1;
      }
      add(keyed[at]!.path, `sample:${keyed[at]!.stratum}`);
    }
  }
  // (b) hot clauses: at most one file over the limit per clause, the cheapest, within the budget.
  let spent = sum([...reasons.keys()].map(sqlCost));
  const hot = new Map<string, PairCost[]>();
  for (const pair of pairs.filter(pair => pair.milliseconds > limitMs)) hot.set(pair.clause, [...(hot.get(pair.clause) ?? []), pair]);
  for (const [clause, heavy] of [...hot].sort((a, b) => compareStrings(a[0], b[0]))) {
    if (heavy.some(pair => reasons.has(pair.path))) continue;
    const pick = heavy.filter(pair => !pair.capped && byPath.has(pair.path)).sort((a, b) => sqlCost(a.path) - sqlCost(b.path) || compareStrings(a.path, b.path))[0];
    if (pick === undefined) continue;
    if (spent + sqlCost(pick.path) > budgetMs) {
      excluded.set(pick.path, `the cheapest heavy file of hot ${clause} (SQL ${seconds(sqlCost(pick.path))}), over what the ${options.sqlBudgetSeconds} s budget has left`);
      continue;
    }
    add(pick.path, `hot:${clause}`);
    spent += sqlCost(pick.path);
  }
  for (const path of reasons.keys()) excluded.delete(path);
  // Verify on the subset itself: cross-file clauses may lose their findings without their evidence files.
  const verify = async () => {
    const result = await timedNative([...reasons.keys()].sort(compareStrings).map(path => byPath.get(path)!));
    const counts = new Map<string, number>();
    for (const violation of result.result.violations) {
      const name = `${linterOf.get(violation.ruleId) ?? violation.ruleId}/${violation.clauseId}`;
      counts.set(name, (counts.get(name) ?? 0) + 1);
    }
    return { counts, wallMs: result.wallMs };
  };
  let verified = await verify();
  const lost = clauses.filter(clause => clause.crossFile && findingFile.has(clause.name) && (verified.counts.get(clause.name) ?? 0) === 0);
  for (const clause of lost) {
    const paths = [...(findings.get(clause.name)?.keys() ?? [])].filter(path => byPath.has(path));
    if (paths.length <= 20 && !paths.some(expensive)) for (const path of paths) add(path, "coverage:cross-file evidence");
  }
  if (lost.length > 0) verified = await verify();
  const coverage: Subset["clauses"] = {};
  for (const clause of clauses) {
    const fullFindings = sum(findings.get(clause.name)?.values() ?? []);
    const subsetFindings = verified.counts.get(clause.name) ?? 0;
    const clean = cleanFile.get(clause.name) ?? null;
    coverage[clause.name] = {
      finding: findingFile.get(clause.name) ?? null,
      clean,
      subsetFindings,
      fullFindings,
      crossFile: clause.crossFile,
      status: fullFindings === 0 ? "no-finding-in-full-tree" : subsetFindings === 0 ? "finding-only-in-full-run" : clean === null ? "no-clean-file" : "covered",
    };
  }
  const previous = existsSync(options.subset) ? subsetSchema.safeParse(JSON.parse(readFileSync(options.subset, "utf8"))) : undefined;
  const files = [...reasons].map(([path, why]) => ({ path, bytes: bytes.get(path)!, sqlMs: Math.round(sqlCost(path)), reasons: [...why].sort(compareStrings) })).sort((a, b) => compareStrings(a.path, b.path));
  const manifest: Subset = {
    format: "rerail.sql-loop-subset/v2",
    generated: new Date().toISOString(),
    corpus: { commit: repository.git.commit, files: repository.files.length, bytes: repository.bytes },
    policies: policiesHash(options.policies),
    selection: { seed: options.seed, sample: options.sample, costs: costed === undefined ? null : `${options.costs.replace(homedir(), "~")} (${costed.measured}, addon ${costed.addon})`, sqlBudgetSeconds: options.sqlBudgetSeconds, fileLimitSeconds: options.fileLimitSeconds },
    files,
    sqlCostMs: costed === undefined ? null : Math.round(sum(files.map(file => file.sqlMs))),
    excluded: [...excluded].map(([path, reason]) => ({ path, reason })).sort((a, b) => compareStrings(a.path, b.path)),
    clauses: coverage,
    // A changed file list invalidates the calibration.
    calibration: previous?.success === true && JSON.stringify(previous.data.files.map(file => file.path)) === JSON.stringify(files.map(file => file.path)) ? previous.data.calibration : null,
  };
  writeFileSync(options.subset, `${JSON.stringify(manifest, null, 2)}\n`);
  const statuses = new Map<string, number>();
  for (const entry of Object.values(coverage)) statuses.set(entry.status, (statuses.get(entry.status) ?? 0) + 1);
  const kinds = new Map<string, number>();
  for (const file of files) for (const kind of new Set(file.reasons.map(reason => reason.startsWith("coverage") ? reason : reason.slice(0, reason.indexOf(":"))))) kinds.set(kind, (kinds.get(kind) ?? 0) + 1);
  console.log(`select: ${files.length} files (${(sum(files.map(file => file.bytes)) / 1e6).toFixed(2)} MB of ${(repository.bytes / 1e6).toFixed(1)} MB); native full ${seconds(full.wallMs)}, subset ${seconds(verified.wallMs)}${manifest.sqlCostMs === null ? "" : `; Σ SQL ${seconds(manifest.sqlCostMs)} (${measured.size} files measured, the rest estimated) of the ${options.sqlBudgetSeconds} s budget; ${manifest.excluded.length} files left out for cost`}`);
  console.log(`  clauses: ${[...statuses].map(([status, count]) => `${count} ${status}`).join(", ")}`);
  console.log(`  files by reason: ${[...kinds].map(([kind, count]) => `${kind} ${count}`).join(", ")}`);
  console.log(`  wrote ${options.subset}${manifest.calibration === null ? " (uncalibrated: run `loop.ts run`, then `loop.ts calibrate`)" : ""}`);
  return 0;
}

// ---------------------------------------------------------------------------------------------------------------------

async function main(): Promise<number> {
  let options: Options;
  try {
    options = parseOptions(process.argv.slice(2));
    if (options.command === "unit-child") return await unitChild(options);
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
