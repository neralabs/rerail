/**
 * The conformance check spread over processes and machines. Each pack's scoring is a list of jobs (`planPack`), which
 * any process making the same plan can run; a report is made from all their results, in order, so it is the report one
 * process running every job writes.
 *
 * `--check --shard <k>/<n>` runs shard k of n of every pack's jobs: within each source of each pack (its fixture units,
 * then its sample rules), the k-th of n runs of consecutive jobs of about equal weight. It writes the results under a
 * directory, one file per pack (`<pack>/<k>-of-<n>.bin`), so CI's shards each score a share of every pack on a machine of
 * their own. `--check --results <dir>` then makes every pack's report from the results all shards wrote, and judges the
 * gated tool-packs and the packs' own rules. A plain `--check` is both, as shard 1 of 1, into a temporary directory.
 *
 * On each machine the work goes to worker processes (worker.ts), one per core and per 5 GiB of memory (`WorkerPool`):
 * the engine evaluates one workspace at a time per process. A shard's jobs go out in chunks, as workers come free.
 */
import { fork, type ChildProcess } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { availableParallelism, totalmem } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { deserialize, serialize } from "node:v8";
import type { DocumentGates, ToolPackVerdict } from "./gates.js";
import { gatedToolPacks } from "./manifest.js";
import { loadPack, RERAIL_ROOT } from "./pack.js";
import type { T4Report } from "./report.js";
import type { Job, JobResult, RunOptions } from "./runner.js";

/** The options every process plans with: a function (the samples-directory test seam) cannot travel. */
export type PlanOptions = Omit<RunOptions, "samplesDirectory">;

/**
 * What a worker is asked. `jobs` plans the pack, compiling its rules; `evaluate` runs jobs, planning the pack if this
 * worker has not, with the rules the `jobs` plan compiled (`compiles`), compiling each only when a job needs it.
 */
export type WorkerRequest =
  | { kind: "jobs"; directory: string; options: PlanOptions }
  | { kind: "evaluate"; directory: string; options: PlanOptions; compiles: string[]; jobs: number[] }
  | { kind: "report"; directory: string; options: PlanOptions; results: string }
  | { kind: "documents"; directory: string; tool: string }
  | { kind: "judge"; directory: string; report: T4Report; tool: string; documents: DocumentGates }
  | { kind: "own"; directory: string };

/** A worker's answer, of its request's `kind`, or the error that stopped it. */
export type WorkerResponse =
  | { kind: "jobs"; jobs: JobOutline[]; compiles: string[] }
  | { kind: "evaluate"; results: JobResult[] }
  | { kind: "report"; report: T4Report }
  | { kind: "documents"; documents: DocumentGates }
  | { kind: "judge"; verdict: ToolPackVerdict }
  | { kind: "error"; error: string };

/** A shard of the jobs: the `index`-th (1-based) of `count`. */
export interface Shard {
  index: number;
  count: number;
}

/** `<k>/<n>`, with 1 ≤ k ≤ n. */
export function parseShard(text: string): Shard {
  const match = /^(\d+)\/(\d+)$/u.exec(text);
  const shard = match === null ? undefined : { index: Number(match[1]), count: Number(match[2]) };
  if (shard === undefined || shard.count < 1 || shard.index < 1 || shard.index > shard.count) throw new Error(`--shard takes <k>/<n> with 1 ≤ k ≤ n, not ${text}`);
  return shard;
}

/** What the parent knows of a job, to share jobs out: everything but running it. */
export type JobOutline = Pick<Job, "source" | "weight" | "group">;

/** Runs of jobs each shard takes of each source, dealt in turn, so work clustered in a source spreads over the shards. */
const STRIPES = 4;

/**
 * The jobs of `shard`: in each run of consecutive jobs of one source, its jobs grouped by `group` (groups in order of
 * first appearance, jobs in order within them), cut into `count × STRIPES` stripes of about equal weight (a job going to
 * the stripe its weight's midpoint falls in), dealt to the shards in turn. A shard so runs few groups, and compiles
 * little for them.
 */
export function shardJobs(jobs: readonly JobOutline[], shard: Shard): number[][] {
  const blocks: number[][] = [];
  for (const [index, job] of jobs.entries()) {
    if (index === 0 || jobs[index - 1]!.source !== job.source) blocks.push([]);
    blocks.at(-1)!.push(index);
  }
  const stripes = shard.count * STRIPES;
  return blocks.map(block => {
    const groups = new Map<string, number[]>();
    for (const index of block) {
      const group = groups.get(jobs[index]!.group);
      if (group === undefined) groups.set(jobs[index]!.group, [index]);
      else group.push(index);
    }
    const grouped = [...groups.values()].flat();
    const total = grouped.reduce((sum, index) => sum + jobs[index]!.weight, 0);
    let before = 0;
    return grouped.filter(index => {
      const weight = jobs[index]!.weight;
      const stripe = total === 0 ? 0 : Math.min(stripes - 1, Math.floor(((before + weight / 2) / total) * stripes));
      before += weight;
      return stripe % shard.count === shard.index - 1;
    });
  });
}

interface ResultsFile {
  /** How many jobs the pack's plan has, so results from another plan are refused. */
  planned: number;
  jobs: number[];
  results: JobResult[];
}

export function writeResults(directory: string, pack: string, shard: Shard, planned: number, jobs: number[], results: JobResult[]): void {
  mkdirSync(join(directory, pack), { recursive: true });
  writeFileSync(join(directory, pack, `${shard.index}-of-${shard.count}.bin`), serialize({ planned, jobs, results } satisfies ResultsFile));
}

/** Every job's result for `pack` from the shard files under `directory`, in job order; each job must have exactly one. */
export function readResults(directory: string, pack: string, planned: number): JobResult[] {
  const results: (JobResult | undefined)[] = Array.from({ length: planned });
  const files = readdirSync(join(directory, pack)).filter(file => file.endsWith(".bin")).sort();
  for (const file of files) {
    const read = deserialize(readFileSync(join(directory, pack, file))) as ResultsFile;
    if (read.planned !== planned) throw new Error(`${pack}/${file}: results of a plan of ${read.planned} job(s); this one has ${planned}`);
    for (const [at, index] of read.jobs.entries()) {
      if (results[index] !== undefined) throw new Error(`${pack}/${file}: job ${index} has results from another shard too`);
      results[index] = read.results[at];
    }
  }
  const missing = results.findIndex(result => result === undefined);
  if (missing !== -1) throw new Error(`${pack}: no shard under ${directory} has results for job ${missing} of ${planned}`);
  return results as JobResult[];
}

const WORKER = fileURLToPath(new URL("./worker.ts", import.meta.url));

interface Pending {
  request: WorkerRequest;
  resolve(response: WorkerResponse): void;
  reject(error: Error): void;
}

/**
 * The memory one worker is given: a worker can hold a few gigabytes of compiled rules (the node pack's builtins rule
 * alone compiles to about one), and a machine that pages runs slower than one with a worker fewer.
 */
const WORKER_MEMORY = 5 * 1024 ** 3;

/**
 * Worker processes, one per core and per `WORKER_MEMORY` of the machine's memory, each running one request at a time;
 * requests wait in order for a free one, which takes the first for a pack it has planned already, if any, since
 * planning a pack compiles its rules. The first failure (a worker's error, or a worker that exits) fails every request
 * not answered yet, and stops the workers.
 */
export class WorkerPool {
  readonly #idle: ChildProcess[] = [];
  readonly #children: ChildProcess[] = [];
  readonly #busy = new Map<ChildProcess, Pending>();
  readonly #planned = new Map<ChildProcess, Set<string>>();
  readonly #waiting: Pending[] = [];
  #failure: Error | undefined;

  constructor(readonly size = Math.max(1, Math.min(availableParallelism(), Math.floor(totalmem() / WORKER_MEMORY)))) {
    for (let index = 0; index < size; index += 1) {
      // `--expose-gc`: a worker collects after each request (worker.ts).
      const child = fork(WORKER, { execArgv: [...process.execArgv, "--expose-gc"], serialization: "advanced" });
      child.on("message", (response: WorkerResponse) => {
        const pending = this.#busy.get(child);
        this.#busy.delete(child);
        this.#idle.push(child);
        if (response.kind !== "error") pending?.resolve(response);
        else {
          const error = new Error(response.error);
          pending?.reject(error);
          this.#fail(error);
        }
        this.#next();
      });
      child.once("exit", (code, signal) => this.#fail(new Error(`a conformance worker exited with ${signal ?? code}`)));
      this.#children.push(child);
      this.#planned.set(child, new Set());
      this.#idle.push(child);
    }
  }

  /** The pack's jobs, and the rules that compile (`PackPlan.compiles`). */
  async jobs(directory: string, options: PlanOptions): Promise<{ jobs: JobOutline[]; compiles: string[] }> {
    const response = await this.#run({ kind: "jobs", directory, options });
    if (response.kind !== "jobs") throw unexpected(response);
    return { jobs: response.jobs, compiles: response.compiles };
  }

  async evaluate(directory: string, options: PlanOptions, compiles: string[], jobs: number[]): Promise<JobResult[]> {
    const response = await this.#run({ kind: "evaluate", directory, options, compiles, jobs });
    if (response.kind !== "evaluate") throw unexpected(response);
    return response.results;
  }

  /** The pack's report, from the results every shard wrote under `results`. */
  async report(directory: string, options: PlanOptions, results: string): Promise<T4Report> {
    const response = await this.#run({ kind: "report", directory, options, results });
    if (response.kind !== "report") throw unexpected(response);
    return response.report;
  }

  /** The gates of the tool-pack's documents alone (`judgeDocuments`), which need no report. */
  async documents(directory: string, tool: string): Promise<DocumentGates> {
    const response = await this.#run({ kind: "documents", directory, tool });
    if (response.kind !== "documents") throw unexpected(response);
    return response.documents;
  }

  async judge(directory: string, report: T4Report, tool: string, documents: DocumentGates): Promise<ToolPackVerdict> {
    const response = await this.#run({ kind: "judge", directory, report, tool, documents });
    if (response.kind !== "judge") throw unexpected(response);
    return response.verdict;
  }

  /** The verdict on the pack's own rules (`judgeOwnRules`), which need no report. */
  async own(directory: string): Promise<ToolPackVerdict> {
    const response = await this.#run({ kind: "own", directory });
    if (response.kind !== "judge") throw unexpected(response);
    return response.verdict;
  }

  /** Lets the workers exit once they are idle. */
  close(): void {
    for (const child of this.#children) {
      child.removeAllListeners("exit");
      if (child.connected) child.disconnect();
    }
  }

  #run(request: WorkerRequest): Promise<WorkerResponse> {
    const { promise, resolve, reject } = Promise.withResolvers<WorkerResponse>();
    if (this.#failure !== undefined) reject(this.#failure);
    else {
      this.#waiting.push({ request, resolve, reject });
      this.#next();
    }
    return promise;
  }

  /**
   * A free worker takes the first plan asked for, since jobs wait on it; else the first request for a pack it has
   * planned; else the first.
   */
  #next(): void {
    while (this.#failure === undefined && this.#idle.length > 0 && this.#waiting.length > 0) {
      const child = this.#idle.pop()!;
      const planned = this.#planned.get(child)!;
      const plan = this.#waiting.findIndex(({ request }) => request.kind === "jobs");
      const index = plan !== -1 ? plan : this.#waiting.findIndex(({ request }) => planned.has(request.directory));
      const [pending] = this.#waiting.splice(Math.max(0, index), 1);
      planned.add(pending!.request.directory);
      this.#busy.set(child, pending!);
      child.send(pending!.request);
    }
  }

  #fail(error: Error): void {
    if (this.#failure !== undefined) return;
    this.#failure = error;
    for (const { reject } of [...this.#busy.values(), ...this.#waiting.splice(0)]) reject(error);
    this.#busy.clear();
    for (const child of this.#children) {
      child.removeAllListeners("exit");
      child.kill();
    }
  }
}

function unexpected(response: WorkerResponse): Error {
  return new Error(`a conformance worker answered ${response.kind}, not what it was asked`);
}

/**
 * Runs `shard` of every pack's jobs on `pool` and writes their results under `directory`, a pack's as soon as its plan
 * says what its jobs are; resolves to how many jobs it ran. Each source's share goes out in chunks of a quarter of a
 * worker's share or so, the sources' chunks taken in turn.
 */
export async function evaluateShard(pool: WorkerPool, directories: readonly string[], options: PlanOptions, shard: Shard, directory: string): Promise<number> {
  const counts = await Promise.all(
    directories.map(async packDirectory => {
      const { jobs, compiles } = await pool.jobs(packDirectory, options);
      const sources = shardJobs(jobs, shard).map(block => {
        const target = block.reduce((sum, index) => sum + jobs[index]!.weight, 0) / (4 * pool.size);
        const chunks: number[][] = [];
        let weight = Infinity;
        for (const index of block) {
          if (weight >= target) {
            chunks.push([]);
            weight = 0;
          }
          chunks.at(-1)!.push(index);
          weight += jobs[index]!.weight;
        }
        return chunks;
      });
      const turns = Math.max(0, ...sources.map(chunks => chunks.length));
      const chunks = Array.from({ length: turns }, (_, turn) => sources.flatMap(chunksOf => (turn < chunksOf.length ? [chunksOf[turn]!] : []))).flat();
      const evaluated = await Promise.all(chunks.map(chunk => pool.evaluate(packDirectory, options, compiles, chunk)));
      writeResults(directory, loadPack(packDirectory).pack.pack, shard, jobs.length, chunks.flat(), evaluated.flat());
      return chunks.flat().length;
    }),
  );
  return counts.reduce((sum, count) => sum + count, 0);
}

/**
 * Each pack's report from the results under `results`, and the verdicts of its gated tool-packs, in path order, then
 * the verdict on its own rules when it has any. The tool-packs' document gates and the own rules' verdict are asked
 * for with the reports, so they are made while the reports are.
 */
export function reportPacks(
  pool: WorkerPool,
  directories: readonly string[],
  options: PlanOptions,
  results: string,
): { report: Promise<T4Report>; verdicts: Promise<ToolPackVerdict[]> }[] {
  const gated = gatedToolPacks(RERAIL_ROOT);
  return directories.map(packDirectory => {
    const { pack } = loadPack(packDirectory);
    const report = pool.report(packDirectory, options, results);
    const verdicts = Promise.all([
      ...gated
        .filter(entry => entry.pack === pack.pack)
        .map(async ({ tool }) => {
          const documents = pool.documents(packDirectory, tool);
          documents.catch(() => undefined);
          return pool.judge(packDirectory, await report, tool, await documents);
        }),
      ...(pack.own.length === 0 ? [] : [pool.own(packDirectory)]),
    ]);
    // Each is awaited in its turn; one failing meanwhile is not left unhandled.
    report.catch(() => undefined);
    verdicts.catch(() => undefined);
    return { report, verdicts };
  });
}
