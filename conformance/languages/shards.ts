/**
 * The conformance check spread over processes and machines. Each pack's scoring is a list of jobs (`planPack`), which
 * any process making the same plan can run; a report is made from all their results, in order, so it is the report one
 * process running every job writes.
 *
 * `--check --shard <k>/<n>` runs shard k of n of every pack's jobs: within each source of each pack (its fixture units,
 * then its sample rules), the k-th of n runs of consecutive jobs of about equal weight. It writes the results under a
 * directory, one file per pack (`<pack>/<k>-of-<n>.bin`), so CI's shards each score a share of every pack on a machine of
 * their own. `--check --results <dir>` then makes every pack's report from the results all shards wrote, and judges the
 * gated tool-packs. A plain `--check` is both, as shard 1 of 1, into a temporary directory.
 *
 * On each machine the work goes to worker processes (worker.ts), one per core: the engine evaluates one workspace at a
 * time per process. A shard's jobs go out in chunks, as workers come free.
 */
import { fork, type ChildProcess } from "node:child_process";
import { mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { availableParallelism } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { deserialize, serialize } from "node:v8";
import type { ToolPackVerdict } from "./gates.js";
import { gatedToolPacks } from "./manifest.js";
import { loadPack, RERAIL_ROOT } from "./pack.js";
import type { T4Report } from "./report.js";
import type { Job, JobResult, RunOptions } from "./runner.js";

/** The options every process plans with: a function (the samples-directory test seam) cannot travel. */
export type PlanOptions = Omit<RunOptions, "samplesDirectory">;

export type WorkerRequest =
  | { kind: "jobs"; directory: string; options: PlanOptions }
  | { kind: "evaluate"; directory: string; options: PlanOptions; jobs: number[] }
  | { kind: "report"; directory: string; options: PlanOptions; results: string }
  | { kind: "judge"; directory: string; report: T4Report; tool: string };

/** A worker's answer, of its request's `kind`, or the error that stopped it. */
export type WorkerResponse =
  | { kind: "jobs"; jobs: Pick<Job, "source" | "weight">[] }
  | { kind: "evaluate"; results: JobResult[] }
  | { kind: "report"; report: T4Report }
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

/**
 * The jobs of `shard`: in each run of consecutive jobs of one source, the shard's share of about equal weight, a job
 * going to the share its weight's midpoint falls in.
 */
export function shardJobs(jobs: readonly Pick<Job, "source" | "weight">[], shard: Shard): number[][] {
  const blocks: number[][] = [];
  for (const [index, job] of jobs.entries()) {
    if (index === 0 || jobs[index - 1]!.source !== job.source) blocks.push([]);
    blocks.at(-1)!.push(index);
  }
  return blocks.map(block => {
    const total = block.reduce((sum, index) => sum + jobs[index]!.weight, 0);
    let before = 0;
    return block.filter(index => {
      const weight = jobs[index]!.weight;
      const share = total === 0 ? 0 : Math.min(shard.count - 1, Math.floor(((before + weight / 2) / total) * shard.count));
      before += weight;
      return share === shard.index - 1;
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
 * Worker processes, one per core, each running one request at a time; requests wait in order for a free one, which
 * takes the first for a pack it has planned already, if any, since planning a pack compiles its rules. The first
 * failure (a worker's error, or a worker that exits) fails every request not answered yet, and stops the workers.
 */
export class WorkerPool {
  readonly #idle: ChildProcess[] = [];
  readonly #children: ChildProcess[] = [];
  readonly #busy = new Map<ChildProcess, Pending>();
  readonly #planned = new Map<ChildProcess, Set<string>>();
  readonly #waiting: Pending[] = [];
  #failure: Error | undefined;

  constructor(readonly size = availableParallelism()) {
    for (let index = 0; index < size; index += 1) {
      const child = fork(WORKER, { execArgv: process.execArgv, serialization: "advanced" });
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

  async jobs(directory: string, options: PlanOptions): Promise<Pick<Job, "source" | "weight">[]> {
    const response = await this.#run({ kind: "jobs", directory, options });
    if (response.kind !== "jobs") throw unexpected(response);
    return response.jobs;
  }

  async evaluate(directory: string, options: PlanOptions, jobs: number[]): Promise<JobResult[]> {
    const response = await this.#run({ kind: "evaluate", directory, options, jobs });
    if (response.kind !== "evaluate") throw unexpected(response);
    return response.results;
  }

  /** The pack's report, from the results every shard wrote under `results`. */
  async report(directory: string, options: PlanOptions, results: string): Promise<T4Report> {
    const response = await this.#run({ kind: "report", directory, options, results });
    if (response.kind !== "report") throw unexpected(response);
    return response.report;
  }

  async judge(directory: string, report: T4Report, tool: string): Promise<ToolPackVerdict> {
    const response = await this.#run({ kind: "judge", directory, report, tool });
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

  #next(): void {
    while (this.#failure === undefined && this.#idle.length > 0 && this.#waiting.length > 0) {
      const child = this.#idle.pop()!;
      const planned = this.#planned.get(child)!;
      const [pending] = this.#waiting.splice(Math.max(0, this.#waiting.findIndex(({ request }) => planned.has(request.directory))), 1);
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
 * Runs `shard` of every pack's jobs on `pool` and writes their results under `directory`. Each source's share goes out
 * in chunks of a quarter of a worker's share or so, the sources' chunks taken in turn, so they all finish about together.
 */
export async function evaluateShard(pool: WorkerPool, directories: readonly string[], options: PlanOptions, shard: Shard, directory: string): Promise<void> {
  const packs = await Promise.all(
    directories.map(async packDirectory => {
      const jobs = await pool.jobs(packDirectory, options);
      const blocks = shardJobs(jobs, shard).map(block => {
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
      return { packDirectory, pack: loadPack(packDirectory).pack.pack, planned: jobs.length, blocks };
    }),
  );
  const sources = packs.flatMap(({ packDirectory, blocks }) => blocks.map(chunks => chunks.map(jobs => ({ packDirectory, jobs }))));
  const turns = Math.max(0, ...sources.map(chunks => chunks.length));
  const chunks = Array.from({ length: turns }, (_, turn) => sources.flatMap(chunksOf => (turn < chunksOf.length ? [chunksOf[turn]!] : []))).flat();
  const evaluated = await Promise.all(chunks.map(({ packDirectory, jobs }) => pool.evaluate(packDirectory, options, jobs)));
  for (const { packDirectory, pack, planned } of packs) {
    const jobs: number[] = [];
    const results: JobResult[] = [];
    for (const [at, chunk] of chunks.entries()) {
      if (chunk.packDirectory !== packDirectory) continue;
      jobs.push(...chunk.jobs);
      results.push(...evaluated[at]!);
    }
    writeResults(directory, pack, shard, planned, jobs, results);
  }
}

/** Each pack's report from the results under `results`, and the verdicts of its gated tool-packs, in path order. */
export function reportPacks(
  pool: WorkerPool,
  directories: readonly string[],
  options: PlanOptions,
  results: string,
): { report: Promise<T4Report>; verdicts: Promise<ToolPackVerdict[]> }[] {
  const gated = gatedToolPacks(RERAIL_ROOT);
  return directories.map(packDirectory => {
    const pack = loadPack(packDirectory).pack.pack;
    const report = pool.report(packDirectory, options, results);
    const verdicts = report.then(made => Promise.all(gated.filter(entry => entry.pack === pack).map(({ tool }) => pool.judge(packDirectory, made, tool))));
    // Each is awaited in its turn; one failing meanwhile is not left unhandled.
    report.catch(() => undefined);
    verdicts.catch(() => undefined);
    return { report, verdicts };
  });
}
