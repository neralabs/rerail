// `just rerail-sql-loop`: the SQL engine's validation loop over a checked-in subset of the corpus (`loop-subset.json`).
// The reporting clauses run in units: a policy's clauses in chunks (`--group`), with its `define`s, each unit one
// workspace over the subset, on SQL and on native in alternating rounds spread over `--jobs` child processes. A unit
// past its cap is killed; a chunk then splits into its clauses, each alone, so a clause past its cap is a TIMEOUT row
// with its time. Clauses that timed out or ran long in the previous run start alone. The verdict is parity (each
// clause's SQL lines against native's over the subset, apart from native's budget failures), speed (SQL/native CPU and
// wall over the units both finished, the costliest statements, clauses and (clause, file)s, against the previous run),
// a projection to the whole tree, and the whole-program SQL setup (every clause in one program, compiled, and a first
// session), capped. Native's lines over the subset and its whole-tree time are cached per native source hash
// (`~/rerail-bench/data-layer/loop-cache/`); every run is recorded in `~/rerail-bench/data-layer/loop-runs/`.
//
//   loop.ts run        [--rounds N] [--jobs N] [--group N] [--cap S] [--group-cap S] [--clauses a,b] [--program-cap S] [--show N]
//   loop.ts calibrate  [--jobs N] [--cap S] [--group-cap S]: the latest run's units over the full corpus, against its projection
//   loop.ts costs      [--jobs N] [--cap S] [--files-from list]: each file's SQL time with every clause in one program, per-(clause, file) attribution
//   loop.ts select     --from <distilled subset.json>: writes loop-subset.json (the files, why, and per-clause native coverage)
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
/** A clause whose SQL time in the previous run reached this starts alone, so it cannot sink a chunk. */
const ISOLATE_MS = 5000;
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
  /** Clauses per chunk of a policy. */
  group: number;
  /** A clause alone: its cap in seconds (costs: per (clause, file)). */
  capSeconds: number;
  /** A chunk of clauses: its cap in seconds, past which it splits into its clauses. */
  groupCapSeconds: number;
  programCapSeconds: number;
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
  "--addon", "--corpus", "--policies", "--subset", "--rounds", "--jobs", "--group", "--cap", "--group-cap", "--program-cap",
  "--clauses", "--show", "--from", "--files-from", "--output", "--scope",
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
  const scope = values["--scope"] ?? "subset";
  if (scope !== "subset" && scope !== "full" && scope !== "list") throw new UsageError(`--scope takes subset, full or list, not ${scope}`);
  const defaultCap = command === "calibrate" ? "120" : command === "costs" ? "5" : "20";
  return {
    command: command as Options["command"],
    addon: resolve(values["--addon"] ?? process.env.RERAIL_LOOP_ADDON ?? defaultAddon),
    corpus: resolve(values["--corpus"] ?? process.env.RERAIL_DATA_LAYER_CORPUS ?? resolve(benchHome, "nera-data-layer-fce4dc72")),
    policies: resolve(values["--policies"] ?? process.env.RERAIL_DATA_LAYER_POLICIES ?? resolve(stateRoot, "policies")),
    subset: resolve(values["--subset"] ?? defaultSubset),
    rounds: integer("--rounds", values["--rounds"] ?? "2", 1),
    jobs: integer("--jobs", values["--jobs"] ?? "4", 1),
    group: integer("--group", values["--group"] ?? "40", 1),
    capSeconds: integer("--cap", values["--cap"] ?? defaultCap, 1),
    groupCapSeconds: integer("--group-cap", values["--group-cap"] ?? (command === "calibrate" ? "300" : "40"), 1),
    programCapSeconds: integer("--program-cap", values["--program-cap"] ?? "30", 0),
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
});
type UnitRequest = z.input<typeof requestSchema>;
const clauseResultSchema = z.object({
  clause: z.string(),
  lines: z.array(z.string()),
  /** SQL: the clause's statement time in the profile (its share of the unit's time); null on native. */
  sqlMs: z.number().nullable(),
});
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
  }),
]);
type UnitEvent = z.infer<typeof unitEventSchema>;
type UnitDone = Extract<UnitEvent, { kind: "unit" }>;
type ReadyEvent = Extract<UnitEvent, { kind: "ready" }>;

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
    const { unit } = request;
    const compileStarted = performance.now();
    const rules = unit.linter === ALL ? loadRules(engine, options.policies) : [unitRule(engine, options.policies, unit)];
    const compileMs = performance.now() - compileStarted;
    const linterOf = new Map(rules.map(rule => [documentSchema.parse(rule.document).id, rule.linter]));
    const name = (ruleId: string | undefined, clauseId: string) => `${linterOf.get(ruleId ?? "") ?? ruleId}/${clauseId}`;
    const matcher: Matcher = request.engine === "sql" ? { engine: "sql", ...(request.perFile ? { sqlBatchSize: 1 } : {}) } : { engine: "native" };
    const workspace = new RuleWorkspace(engine, rules, matcher);
    try {
      const candidates = request.files === undefined ? files : request.files.map(path => byPath.get(path)!);
      const accepted = candidates.filter(file => workspace.accepts(file.path));
      emit({ kind: "start", id: request.id, files: accepted.map(file => file.path) });
      const load = loadavg()[0]!;
      let wallMs = 0;
      let cpuMs = 0;
      const lines: string[] = [];
      if (request.perFile) {
        if (unit.linter === ALL && request.engine === "sql") {
          // Untimed: the whole program's compile and its session (batches of one), which every file would otherwise wait on first.
          await workspace.apply([{ path: "__rerail_sql_loop_program__.py", language: "python", source: PROGRAM_PROBE_SOURCE }]);
          await workspace.apply([], ["__rerail_sql_loop_program__.py"]);
        }
        for (const file of accepted) {
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
      } else if (accepted.length > 0) {
        const timed = await workspace.apply(accepted);
        ({ wallMs, cpuMs } = timed);
        lines.push(...linesOf(timed.result));
      }
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

/** A unit's outcome on one engine: done, or killed at its cap (TIMEOUT, with its time), or its child died. */
type UnitOutcome =
  | ({ status: "ok"; unit: Unit } & Omit<UnitDone, "kind" | "id">)
  | { status: "timeout" | "error"; unit: Unit; wallMs: number; detail: string };

interface PairCost {
  clause: string;
  path: string;
  milliseconds: number;
  capped: boolean;
}

interface UnitBatch {
  engine: EngineName;
  units: readonly Unit[];
  /** Instead of `units`: units over given files (`costs` deals the files out to the jobs). */
  items?: readonly { unit: Unit; files: string[] }[];
  capSeconds: number;
  groupCapSeconds: number;
  jobs: number;
  /** One file per apply: a capped file is recorded at the cap and the unit goes on without it. */
  perFile?: boolean;
  /** Called after each unit ends, however it ends. */
  onUnit?: (pairs: readonly PairCost[], outcomes: readonly UnitOutcome[]) => void;
  /** A chunk past its cap splits into its clauses (default); `false`: it is one TIMEOUT unit. */
  split?: boolean;
  label: string;
}

/**
 * Runs `batch`'s units, in order, in `jobs` children of `pool`. A unit past its cap kills its child: a chunk of clauses
 * splits into its clauses, each queued alone (its time is reported as split time); a clause alone is a TIMEOUT row.
 */
async function runUnits(pool: UnitPool, batch: UnitBatch): Promise<{ outcomes: UnitOutcome[]; pairs: PairCost[]; wallMs: number; splitMs: number; splits: string[] }> {
  const started = performance.now();
  const queue: { unit: Unit; files?: string[] }[] = batch.items?.map(item => ({ ...item })) ?? batch.units.map(unit => ({ unit }));
  const outcomes: UnitOutcome[] = [];
  const pairs: PairCost[] = [];
  const splits: string[] = [];
  let splitMs = 0;
  let nextId = 0;
  const worker = async () => {
    let child: UnitChild | undefined;
    for (let item = queue.shift(); item !== undefined; item = queue.shift()) {
      const { unit } = item;
      child ??= await pool.acquire();
      const id = nextId++;
      const unitStarted = performance.now();
      const capMs = (unit.clauses.length > 1 ? batch.groupCapSeconds : batch.capSeconds) * 1000;
      child.send({ id, engine: batch.engine, unit, ...(item.files === undefined ? {} : { files: item.files }), perFile: batch.perFile ?? false });
      let deadline = unitStarted + capMs;
      let accepted: string[] = [];
      const finished = new Set<string>();
      let current: string | undefined;
      for (;;) {
        const event = await child.next(deadline - performance.now());
        if (event === "timeout" || event === undefined) {
          child.kill();
          child = undefined;
          const elapsed = performance.now() - unitStarted;
          // Per file, a unit's whole-file times go under its clause (alone) or "*" (several clauses).
          const clause = unit.clauses.length === 1 ? unit.clauses[0]! : ALL;
          if (unit.clauses.length > 1 && batch.split !== false && !batch.perFile) {
            splitMs += elapsed;
            splits.push(unitKey(unit));
            queue.unshift(...unit.clauses.map(name => ({ unit: { linter: unit.linter, clauses: [name] } })));
            process.stderr.write(`  ${batch.label}: ${unitKey(unit)} ${event === "timeout" ? `passed the ${batch.groupCapSeconds} s cap` : "lost its child"}: its ${unit.clauses.length} clauses go alone\n`);
          } else if (event === "timeout" && batch.perFile && current !== undefined) {
            pairs.push({ clause, path: current, milliseconds: capMs, capped: true });
            process.stderr.write(`  ${batch.label}: ${clause} @ ${current} passed the ${batch.capSeconds} s cap\n`);
            queue.unshift({ unit, files: accepted.filter(path => !finished.has(path) && path !== current) });
          } else {
            const status = event === "timeout" ? "timeout" : "error";
            outcomes.push({ status, unit, wallMs: elapsed, detail: status === "timeout" ? `killed at the ${capMs / 1000} s cap` : "the child exited" });
            process.stderr.write(`  ${batch.label}: ${unitKey(unit)} ${status === "timeout" ? `TIMEOUT at ${seconds(elapsed)}` : "ERROR: its child exited"}\n`);
          }
          batch.onUnit?.(pairs, outcomes);
          break;
        }
        if (event.kind === "start" && event.id === id) accepted = event.files;
        else if (event.kind === "file-start" && event.id === id) {
          current = event.path;
          deadline = performance.now() + capMs;
        } else if (event.kind === "file" && event.id === id) {
          finished.add(event.path);
          current = undefined;
          pairs.push({ clause: unit.clauses.length === 1 ? unit.clauses[0]! : ALL, path: event.path, milliseconds: event.wallMs, capped: false });
          if (unit.clauses.length !== 1) for (const entry of event.clauses) pairs.push({ clause: entry.clause, path: event.path, milliseconds: entry.milliseconds, capped: false });
        } else if (event.kind === "unit" && event.id === id) {
          const { kind: _kind, id: _id, ...rest } = event;
          outcomes.push({ ...rest, status: "ok", unit });
          batch.onUnit?.(pairs, outcomes);
          break;
        }
      }
    }
    if (child !== undefined) pool.release(child);
  };
  await Promise.all(Array.from({ length: Math.min(batch.jobs, Math.max(1, queue.length)) }, worker));
  return { outcomes, pairs, wallMs: performance.now() - started, splitMs, splits };
}

// ---------------------------------------------------------------------------------------------------------------------
// The whole-program SQL setup: every clause in one program, compiled, and every worker's session

const programEventSchema = z.object({ kind: z.literal("program"), compileMs: z.number(), setupMs: z.number(), clauses: z.number().int() });

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
    const event: z.infer<typeof programEventSchema> = { kind: "program", compileMs, setupMs, clauses: reportingClauses(rules).length };
    process.stdout.write(`${JSON.stringify(event)}\n`);
  } finally {
    workspace.close();
  }
  return 0;
}

type ProgramOutcome = { status: "ok"; compileMs: number; setupMs: number; clauses: number } | { status: "timeout" | "error"; wallMs: number };

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
  if (found !== undefined) return { status: "ok", compileMs: found.compileMs, setupMs: found.setupMs, clauses: found.clauses };
  return { status: timedOut ? "timeout" : "error", wallMs: performance.now() - started };
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
  /** Per clause: its SQL statement time (SQL only). */
  sqlMs: z.record(z.string(), z.number()).nullable(),
  detail: z.string().nullable(),
});
type UnitRecord = z.infer<typeof unitRecordSchema>;
const roundRecordSchema = z.object({ sql: z.array(unitRecordSchema), native: z.array(unitRecordSchema), sqlPhaseWallMs: z.number(), nativePhaseWallMs: z.number(), splitMs: z.number() });
const runRecordSchema = z.looseObject({
  format: z.literal("rerail.sql-loop-run/v3"),
  started: z.string(),
  addon: z.looseObject({ sha256: z.string() }),
  subsetSha256: z.string(),
  clauseSetSha256: z.string(),
  rounds: z.array(roundRecordSchema),
  /** Per round, over the units both engines finished. */
  totals: z.object({ sqlCpuMs: z.array(z.number()), sqlWallMs: z.array(z.number()), nativeCpuMs: z.array(z.number()), nativeWallMs: z.array(z.number()) }),
  ratio: z.object({ wall: z.number(), cpu: z.number() }),
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
  ? { unit: outcome.unit, status: "ok", wallMs: outcome.wallMs, cpuMs: outcome.cpuMs, detail: null, sqlMs: outcome.clauses.some(clause => clause.sqlMs !== null) ? Object.fromEntries(outcome.clauses.map(clause => [clause.clause, clause.sqlMs ?? 0])) : null }
  : { unit: outcome.unit, status: outcome.status, wallMs: outcome.wallMs, cpuMs: null, sqlMs: null, detail: outcome.detail };

/**
 * The units: clauses that timed out or reached `ISOLATE_MS` in the previous run alone (costliest first), then each
 * policy's other clauses in chunks of `group`.
 */
function planUnits(clauses: readonly Clause[], group: number, previous: RunRecord | undefined): Unit[] {
  const last = new Map<string, number>();
  for (const round of previous?.rounds ?? []) {
    for (const record of round.sql) {
      if (record.status !== "ok") for (const clause of record.unit.clauses) last.set(clause, Infinity);
      else for (const [clause, milliseconds] of Object.entries(record.sqlMs ?? {})) last.set(clause, Math.max(last.get(clause) ?? 0, milliseconds));
    }
  }
  const alone = clauses.filter(clause => (last.get(clause.name) ?? 0) >= ISOLATE_MS).sort((a, b) => last.get(b.name)! - last.get(a.name)! || compareStrings(a.name, b.name));
  const isolated = new Set(alone.map(clause => clause.name));
  const units: Unit[] = alone.map(clause => ({ linter: clause.linter, clauses: [clause.name] }));
  const linters = [...new Set(clauses.map(clause => clause.linter))];
  for (const linter of linters) {
    const rest = clauses.filter(clause => clause.linter === linter && !isolated.has(clause.name)).map(clause => clause.name);
    for (let start = 0; start < rest.length; start += group) units.push({ linter, clauses: rest.slice(start, start + group) });
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
  const previous = runRecords().find(({ record }) => record.subsetSha256 === subsetSha(subset) && record.clauseSetSha256 === clauseSetSha);
  let units = planUnits(clauses, options.group, previous?.record);
  console.log(`sql-loop: ${names.length} clauses in ${units.length} units (${units.filter(unit => unit.clauses.length === 1).length} alone) over ${subset.files.length} subset files; ${options.rounds} rounds, ${options.jobs} jobs; caps ${options.capSeconds} s alone, ${options.groupCapSeconds} s per chunk; background load ${backgroundLoad.map(value => value.toFixed(1)).join(" ")}`);

  const key = nativeCacheKey(options, subset);
  const { cache, measured } = await nativeCache(options, subset, key);
  if (measured) console.log(`  native cache: measured (whole tree ${seconds(cache.whole.full.wallMs)} wall)`);
  const pool = new UnitPool(options, "subset");
  const rounds: { sql: UnitOutcome[]; native: UnitOutcome[]; sqlWallMs: number; nativeWallMs: number; splitMs: number; splits: string[] }[] = [];
  const timedOut = new Set<string>();
  for (let round = 1; round <= options.rounds; round += 1) {
    // SQL first: its splits and TIMEOUTs fix the units native then runs, and later rounds run without the TIMEOUTs.
    const sql = await runUnits(pool, { engine: "sql", units: units.filter(unit => !timedOut.has(unitKey(unit))), capSeconds: options.capSeconds, groupCapSeconds: options.groupCapSeconds, jobs: options.jobs, label: `sql ${round}` });
    for (const outcome of sql.outcomes) if (outcome.status !== "ok") timedOut.add(unitKey(outcome.unit));
    units = [...sql.outcomes.map(outcome => outcome.unit), ...units.filter(unit => timedOut.has(unitKey(unit)) && !sql.outcomes.some(outcome => unitKey(outcome.unit) === unitKey(unit)))];
    const native = await runUnits(pool, { engine: "native", units: units.filter(unit => !timedOut.has(unitKey(unit))), capSeconds: 120, groupCapSeconds: 120, jobs: options.jobs, label: `native ${round}` });
    rounds.push({ sql: sql.outcomes, native: native.outcomes, sqlWallMs: sql.wallMs, nativeWallMs: native.wallMs, splitMs: sql.splitMs, splits: sql.splits });
    process.stderr.write(`  round ${round}: SQL ${seconds(sql.wallMs)}${sql.splits.length > 0 ? ` (${sql.splits.length} chunks split, ${seconds(sql.splitMs)} lost)` : ""}, native ${seconds(native.wallMs)} (wall, ${options.jobs} jobs)\n`);
  }
  pool.close();
  const program = options.programCapSeconds > 0 ? await runProgram(options) : undefined;

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
  const timeouts = names.filter(clause => !sqlLines.has(clause));
  console.log(`PARITY ${differing.length === 0 ? "OK" : "DIFFERS"}: ${parities.length} clauses compared (${sum(parities.map(parity => parity.compared))} native lines), ${differing.length} differ; ${timeouts.length} not compared (TIMEOUT); ${sum(parities.map(parity => parity.budgetExcluded.length))} (clause, file)s excluded for native budget failures; SQL refused ${refused.size} clauses`);
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
      timeoutMs: sum(round.sql.flatMap(outcome => (outcome.status === "ok" ? [] : [outcome.wallMs]))),
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
  // Rounds compare like for like from round 2 on (round 1 also split chunks).
  const comparable = totals.slice(rounds[0]!.splits.length > 0 ? 1 : 0);
  if (spread(comparable.map(total => total.sqlCpuMs)) > SPREAD_LIMIT) untrusted.push(`SQL rounds spread ${percent(spread(comparable.map(total => total.sqlCpuMs)))} in CPU`);
  if (spread(comparable.map(total => total.nativeCpuMs)) > SPREAD_LIMIT) untrusted.push(`native rounds spread ${percent(spread(comparable.map(total => total.nativeCpuMs)))} in CPU`);
  if (comparable.length < 2) untrusted.push(`${comparable.length} comparable round(s): spread unknown`);
  if (timeouts.length > 0) untrusted.push(`${timeouts.length} TIMEOUT clauses outside the ratio`);
  const unitLoads = rounds.flatMap(round => round.sql.flatMap(outcome => (outcome.status === "ok" ? [outcome.load] : [])));
  console.log("");
  console.log(`SPEED${untrusted.length > 0 ? ` (UNTRUSTED: ${untrusted.join("; ")})` : " (trusted)"}`);
  for (const [index, total] of totals.entries()) {
    const round = rounds[index]!;
    console.log(`  round ${index + 1}: over ${total.units} units both finished: SQL ${seconds(total.sqlCpuMs)} CPU, ${seconds(total.sqlWallMs)} wall; native ${seconds(total.nativeCpuMs)} CPU, ${seconds(total.nativeWallMs)} wall; TIMEOUTs ${seconds(total.timeoutMs)}; phases SQL ${seconds(round.sqlWallMs)}, native ${seconds(round.nativeWallMs)}${round.splits.length > 0 ? `; ${round.splits.length} chunks split (${seconds(round.splitMs)})` : ""}`);
  }
  console.log(`  SQL/native: CPU ${ratio.cpu.toFixed(2)}×, wall ${ratio.wall.toFixed(2)}× (medians of the rounds' sums); load during SQL units median ${median(unitLoads).toFixed(1)}, max ${Math.max(0, ...unitLoads).toFixed(1)}`);
  const lastOutcomes = new Map<string, UnitOutcome>();
  for (const round of rounds) for (const outcome of round.sql) lastOutcomes.set(unitKey(outcome.unit), outcome);
  const okOutcomes = [...lastOutcomes.values()].filter((outcome): outcome is Extract<UnitOutcome, { status: "ok" }> => outcome.status === "ok");
  if (timeouts.length > 0) {
    console.log("  TIMEOUT:");
    for (const outcome of [...lastOutcomes.values()]) if (outcome.status !== "ok") console.log(`    ${outcome.status.toUpperCase().padEnd(7)} ${seconds(outcome.wallMs).padStart(9)}  ${unitKey(outcome.unit)}: ${outcome.detail}`);
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
    const perRound = rounds.flatMap(round => round.sql.flatMap(outcome => (outcome.status === "ok" ? outcome.clauses.filter(entry => entry.clause === clause).map(entry => entry.sqlMs ?? 0) : outcome.unit.clauses.length === 1 && outcome.unit.clauses[0] === clause ? [outcome.wallMs] : [])));
    return { clause, milliseconds: median(perRound), timeout: !sqlLines.has(clause) };
  }).filter(entry => !Number.isNaN(entry.milliseconds)).sort((a, b) => b.milliseconds - a.milliseconds);
  const clauseFiles = okOutcomes.flatMap(outcome => outcome.clauseFiles).sort((a, b) => b.milliseconds - a.milliseconds);
  console.log("  top statements by SQL time (Σ over units and workers, last round):");
  for (const statement of topStatements.slice(0, 10)) console.log(`    ${seconds(statement.milliseconds).padStart(9)} ${String(statement.rows).padStart(10)} rows ${String(statement.runs).padStart(6)} runs  [${statement.units[0]}] ${statement.sql.replace(/\s+/g, " ").slice(0, 140)}`);
  console.log("  top clauses by SQL time (statement time attributed to the clause, median over rounds; TIMEOUT: its time):");
  for (const entry of clauseTimes.slice(0, options.show)) console.log(`    ${seconds(entry.milliseconds).padStart(9)}${entry.timeout ? " TIMEOUT" : ""}  ${entry.clause}`);
  console.log("  top (clause, file) by SQL time (a batch's time split evenly over its files):");
  for (const entry of clauseFiles.slice(0, 10)) console.log(`    ${seconds(entry.milliseconds).padStart(9)}  ${entry.clause} @ ${entry.path}`);

  // Against the previous run on this subset and clause set
  if (previous !== undefined) {
    const before = previous.record;
    console.log("");
    console.log(`AGAINST ${previous.path.replace(homedir(), "~")} (${before.started}):`);
    console.log(`  SQL CPU ${seconds(median(before.totals.sqlCpuMs))} → ${seconds(median(sqlCpu))} (${percent(median(sqlCpu) / median(before.totals.sqlCpuMs) - 1)}); SQL/native CPU ${before.ratio.cpu.toFixed(2)}× → ${ratio.cpu.toFixed(2)}×, wall ${before.ratio.wall.toFixed(2)}× → ${ratio.wall.toFixed(2)}× (units differ where chunks split)`);
    const old = new Map<string, number>();
    for (const round of before.rounds.slice(-1)) {
      for (const record of round.sql) {
        if (record.status === "ok") for (const [clause, milliseconds] of Object.entries(record.sqlMs ?? {})) old.set(clause, milliseconds);
        else old.set(record.unit.clauses[0]!, record.wallMs);
      }
    }
    const deltas = clauseTimes.map(entry => ({ ...entry, before: old.get(entry.clause) ?? 0 })).map(entry => ({ ...entry, delta: entry.milliseconds - entry.before })).sort((a, b) => Math.abs(b.delta) - Math.abs(a.delta));
    console.log("  largest clause changes:");
    for (const entry of deltas.slice(0, 8)) console.log(`    ${entry.delta >= 0 ? "+" : ""}${seconds(entry.delta).padStart(9)}  ${entry.clause} (${seconds(entry.before)} → ${seconds(entry.milliseconds)})`);
    const wasTimeout = new Set(before.rounds.flatMap(round => round.sql.flatMap(record => (record.status === "ok" ? [] : record.unit.clauses))));
    const newTimeouts = timeouts.filter(clause => !wasTimeout.has(clause));
    if (newTimeouts.length > 0) console.log(`  new TIMEOUTs: ${newTimeouts.join(", ")}`);
  }

  // Projection: native's whole tree times the subset's SQL/native CPU ratio, corrected by the calibration.
  const correction = subset.calibration?.correction ?? 1;
  const projectedRatio = ratio.cpu * correction;
  const projection = { ratio: projectedRatio, correction, error: subset.calibration?.error ?? null, nativeWholeWallMs: cache.whole.full.wallMs, sqlWholeWallMs: cache.whole.full.wallMs * projectedRatio };
  console.log("");
  console.log(`PROJECTION: whole-tree SQL ≈ ${seconds(projection.sqlWholeWallMs)} wall = native ${seconds(cache.whole.full.wallMs)} × ${projectedRatio.toFixed(2)}${subset.calibration === null ? " — UNCALIBRATED" : ` (subset ratio × correction ${correction.toFixed(2)}, error ±${(subset.calibration.error * 100).toFixed(0)}%, calibrated ${subset.calibration.date})`}; target ≤ 1.10× = ${seconds(cache.whole.full.wallMs * 1.1)}${timeouts.length > 0 ? `; ${timeouts.length} TIMEOUT clauses not in it (a lower bound)` : ""}; the whole-program setup is not in it (PROGRAM)`);
  if (program !== undefined) {
    console.log("");
    console.log(program.status === "ok"
      ? `PROGRAM: ${program.clauses} clauses in one SQL program: rules compiled in ${seconds(program.compileMs)}; a first batch per worker (SQL program compile and every session's tables) ${seconds(program.setupMs)}`
      : `PROGRAM ${program.status.toUpperCase()}: compiling every clause into one SQL program and opening the sessions ${program.status === "timeout" ? `passed the ${options.programCapSeconds} s cap` : "failed"} (${seconds(program.wallMs)}); the whole-tree run pays this before any file`);
  }

  // Record
  const record = {
    format: "rerail.sql-loop-run/v3" as const,
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
    options: { rounds: options.rounds, jobs: options.jobs, group: options.group, capSeconds: options.capSeconds, groupCapSeconds: options.groupCapSeconds, clauses: options.clauses, programCapSeconds: options.programCapSeconds },
    load: { background: backgroundLoad, unitsMedian: median(unitLoads), unitsMax: Math.max(0, ...unitLoads) },
    rounds: rounds.map(round => ({ sql: round.sql.map(unitRecord), native: round.native.map(unitRecord), sqlPhaseWallMs: round.sqlWallMs, nativePhaseWallMs: round.nativeWallMs, splitMs: round.splitMs })),
    totals: { sqlCpuMs: sqlCpu, sqlWallMs: sqlWall, nativeCpuMs: nativeCpu, nativeWallMs: nativeWall },
    ratio,
    parity: { equal: differing.length === 0, compared: parities.length, differing, timeouts, refused: [...refused].map(([clause, reason]) => ({ clause, reason })) },
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
  console.log(`VERDICT ${verdict}: parity ${differing.length === 0 ? "ok" : `${differing.length} clauses differ`}${timeouts.length > 0 ? `, ${timeouts.length} TIMEOUT` : ""}; SQL/native CPU ${ratio.cpu.toFixed(2)}×, wall ${ratio.wall.toFixed(2)}×${untrusted.length > 0 ? " (untrusted timing)" : ""}; projected whole tree ${projectedRatio.toFixed(2)}×; ${seconds(performance.now() - started)} for the run; record ${recordPath.replace(homedir(), "~")}`);
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
  // Costliest first by the subset's SQL time.
  missing.sort((a, b) => (subsetSql.get(unitKey(b))!.cpuMs ?? 0) - (subsetSql.get(unitKey(a))!.cpuMs ?? 0));
  console.log(`calibrate: ${units.length} units of ${latest.path.replace(homedir(), "~")}; ${missing.length} to run over the full corpus (${options.jobs} jobs, caps ${options.capSeconds} s alone, ${options.groupCapSeconds} s per chunk, chunks do not split); load ${loadavg()[0]!.toFixed(1)}`);
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
    // A chunk past its cap stays one TIMEOUT unit here: it is left out of the calibration, not split.
    const sql = await runUnits(pool, { engine: "sql", units: missing, capSeconds: options.capSeconds, groupCapSeconds: options.groupCapSeconds, jobs: options.jobs, split: false, label: "sql full", onUnit: (_pairs, outcomes) => flush(outcomes, []) });
    const finished = sql.outcomes.filter(outcome => outcome.status === "ok" && missing.some(unit => unitKey(unit) === unitKey(outcome.unit))).map(outcome => outcome.unit);
    const native = await runUnits(pool, { engine: "native", units: finished, capSeconds: 300, groupCapSeconds: 300, jobs: options.jobs, label: "native full" });
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
  const items = Array.from({ length: Math.min(options.jobs, ordered.length) }, (_, job) => ({ unit: { linter: ALL, clauses: [] }, files: ordered.filter((_, index) => index % options.jobs === job) }));
  console.log(`costs: ${paths.length} files, every clause in one SQL program, ${items.length} jobs, ${options.capSeconds} s cap per file; load ${loadavg()[0]!.toFixed(1)}`);
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
  const result = await runUnits(pool, { engine: "sql", units: [], items, capSeconds: options.capSeconds, groupCapSeconds: options.capSeconds, jobs: options.jobs, perFile: true, label: "costs", onUnit: pairs => write(pairs) });
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
