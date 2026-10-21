// `just rerail-workspaces <command>`: peak memory and latency of the code-rule workspace over many repositories in one
// process, on the native or the SQL engine (benchmark/README.md). A port of Tempo's `benchmark:rerailer` (nera-landing
// PR #2622), which measured native's 564 MiB peak over 47 nera-data-layer workspaces.
//
//   setup --data-layer <nera-data-layer clone> --workspaces <dir>   adds SUBSET's checkouts under <dir>
//   run --engine native|sql --rules <dir> [--rules <dir> …] --workspaces <dir> --output <file.json>
//   compare <native.json> <sql.json>                                prints the two runs side by side
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { opendir, readFile, stat } from "node:fs/promises";
import { cpus, loadavg, release, tmpdir } from "node:os";
import { dirname, join, relative, resolve, sep } from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { Worker } from "node:worker_threads";
import { z } from "zod";
import {
  encodeCodeRuleWorkspaceRequest,
  IGNORED_DIRECTORIES,
  languageFor,
  loadNativeBinding,
  type NativeCodeRuleDslWorkspace,
  type NativeCompiledCodeRuleDsl,
  REPOSITORY_INDEX_OPTIONS,
  resolveNativeBindingPath,
} from "../../src/engine/index.js";

const here = dirname(fileURLToPath(import.meta.url));
const rerailRoot = resolve(here, "../..");
const capabilitiesPath = resolve(rerailRoot, "native/src/code_rule_dsl/capabilities.json");
const RULE_FORMAT = "rerail.policy/v1";
/** The page size the enforcer reads a repository audit's results in (`RESULT_PAGE_BYTES`). */
const RESULT_PAGE_BYTES = 256 * 1024;
/** The enforcer applies a single small edit inline (`tryApply`) below this many characters. */
const INLINE_EDIT_CHARACTERS = 4_096;
/** The enforcer's walk without a content store applies batches of 32 files, closed at 1 MiB of source. */
const AUDIT_BATCH_FILES = 32;
const AUDIT_CHUNK_BYTES = 1024 * 1024;
const SAMPLE_INTERVAL_MS = 10;
const MIB = 1024 * 1024;

class UsageError extends Error {}

/**
 * Eight of the 47 nera-data-layer workspaces #2622 measured (each a checkout at the commit given): numbers
 * 1 + round(k × 46 / 7) for k = 0…7, from the largest workspace (01, 5,726 Python files) to the smallest (47, 5,491).
 */
const SUBSET = [
  { name: "01", commit: "fce4dc7208c0b892c11a549c8be43fc285580d6b" },
  { name: "08", commit: "820861353124b50c27f2785143951b37ccbcb7ea" },
  { name: "14", commit: "5a819aee1513df3fac434175b840fde5bab48b8e" },
  { name: "21", commit: "c9016b4b45546a48d95ee5d3b8311f3be27b1fb6" },
  { name: "27", commit: "ca6116dbc6cdc066ce8f4ec3e54d8df51e8df1a1" },
  { name: "34", commit: "c4f94920613706583b39153a26728e2248839590" },
  { name: "40", commit: "17562322111407ccfdcdf148e4a9e09afc6f1730" },
  { name: "47", commit: "e99ce662f0e90653a777b321138a0182271bf43d" },
] as const;

/**
 * The production-path loop's workload (`prod-path`): workspace 01 walked as the enforcer walks it without a store, on a
 * fixed sample of its files, then a few edits.
 */
const PROD_PATH = { only: "01", sample: 40, edits: 8 } as const;

function git(cwd: string, ...args: string[]): string {
  const result = spawnSync("git", ["-C", cwd, ...args], { encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git -C ${cwd} ${args.join(" ")}: ${result.stderr.trim()}`);
  return result.stdout.trim();
}

// ---------------------------------------------------------------------------------------------------------------------
// Peak RSS: a worker thread samples the process's resident set every SAMPLE_INTERVAL_MS, so a peak inside a long
// synchronous native call on the main thread (a rule compile, `addCodeRule`) is seen too. Slot 0 is the run's peak,
// slot 1 the current phase's, which the main thread resets.

class RssSampler {
  readonly #slots = new BigInt64Array(new SharedArrayBuffer(16));
  readonly #worker: Worker;

  constructor() {
    this.#worker = new Worker(
      `const { workerData } = require("node:worker_threads");
       const slots = workerData;
       const sample = () => {
         const rss = BigInt(process.memoryUsage.rss());
         for (let slot = 0; slot < 2; slot += 1) if (rss > Atomics.load(slots, slot)) Atomics.store(slots, slot, rss);
       };
       sample();
       setInterval(sample, ${SAMPLE_INTERVAL_MS});`,
      { eval: true, workerData: this.#slots },
    );
    this.#worker.unref();
  }

  /** Starts a phase; returns the peak of the phase it ends. */
  phase(): number {
    const peak = Number(Atomics.load(this.#slots, 1));
    Atomics.store(this.#slots, 1, BigInt(process.memoryUsage.rss()));
    return Math.max(peak, process.memoryUsage.rss());
  }

  get peak(): number {
    return Math.max(Number(Atomics.load(this.#slots, 0)), process.memoryUsage.rss());
  }

  async stop(): Promise<void> {
    await this.#worker.terminate();
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// Rules

interface LoadedRules {
  compiled: NativeCompiledCodeRuleDsl[];
  ids: string[];
  /** Rule documents the engine does not compile, with its reason; they are left out of the run. */
  rejected: Array<{ path: string; id: string; reason: string }>;
  sha256: string;
}

function ruleFiles(directory: string): string[] {
  return readdirSync(directory, { recursive: true, withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
    .map(entry => join(entry.parentPath, entry.name))
    .sort();
}

/** Every `rerail.policy/v1` document under each directory (recursively, other JSON skipped), compiled once. */
function loadRules(directories: readonly string[]): LoadedRules {
  const binding = loadNativeBinding();
  const capabilities = readFileSync(capabilitiesPath, "utf8");
  const hash = createHash("sha256");
  const result: LoadedRules = { compiled: [], ids: [], rejected: [], sha256: "" };
  const seen = new Set<string>();
  for (const directory of directories)
    for (const path of ruleFiles(directory)) {
      const source = readFileSync(path, "utf8");
      let document: { format?: unknown; id?: unknown };
      try {
        document = JSON.parse(source) as typeof document;
      } catch {
        continue;
      }
      if (document.format !== RULE_FORMAT || typeof document.id !== "string") continue;
      if (seen.has(document.id)) throw new Error(`Rule ${document.id} appears twice (${path})`);
      seen.add(document.id);
      hash.update(source);
      try {
        result.compiled.push(binding.CodeRuleDslRuntime.compile(source, "json", capabilities));
        result.ids.push(document.id);
      } catch (error) {
        result.rejected.push({ path, id: document.id, reason: error instanceof Error ? error.message : String(error) });
      }
    }
  if (result.compiled.length === 0) throw new UsageError(`No rule compiled under ${directories.join(", ")}`);
  result.sha256 = `sha256:${hash.digest("hex")}`;
  return result;
}

// ---------------------------------------------------------------------------------------------------------------------
// One workspace: the enforcer's stored repository audit (attach the content store, index the Git checkout chunk by
// chunk, the final quiet apply, record the revision, read the result pages), then edits to the resident workspace as
// the enforcer applies them, then `evictInactive` and an idle wait.

const profileSchema = z.object({
  files: z.number().int(),
  evaluatedFiles: z.number().int(),
  evaluatedClauses: z.number().int(),
  restoredLocalResults: z.number().int(),
  storageReadNanoseconds: z.number(),
  storageManifestNanoseconds: z.number(),
  storageWriteNanoseconds: z.number(),
});
const evaluationSchema = z.object({
  metrics: z.object({ files: z.number().int() }).passthrough(),
  profile: profileSchema.omit({ files: true }).passthrough().optional(),
}).passthrough();
const chunkSchema = z.object({
  revision: z.number().int(),
  done: z.boolean(),
  overBudget: z.boolean(),
  discoveredFiles: z.number().int(),
  restoredFiles: z.number().int(),
  readFiles: z.number().int(),
  reusableDirectories: z.number().int(),
  evaluation: evaluationSchema.nullable(),
  failures: z.array(z.object({ code: z.string() })),
}).passthrough();
const pageSchema = z.object({
  cursor: z.number().int(),
  done: z.boolean(),
  violations: z.array(z.unknown()),
  failures: z.array(z.object({ code: z.string() }).passthrough()),
}).passthrough();
const applySchema = z.object({ revision: z.number().int(), evaluation: evaluationSchema.optional() }).passthrough();

/**
 * What a scan evaluated and what it reused from stored results, summed over its applies (`RERAIL_CODE_RULE_PROFILE`'s
 * counters): `files` went through an evaluation job, `evaluatedFiles` of them ran at least one clause (the rest were
 * answered wholly by stored results), `restoredLocalResults` clause results were restored instead of evaluated.
 */
type Reuse = z.infer<typeof profileSchema>;

function emptyReuse(): Reuse {
  return { files: 0, evaluatedFiles: 0, evaluatedClauses: 0, restoredLocalResults: 0, storageReadNanoseconds: 0, storageManifestNanoseconds: 0, storageWriteNanoseconds: 0 };
}

function addReuse(into: Reuse, evaluation: z.infer<typeof evaluationSchema> | null | undefined): void {
  if (evaluation === null || evaluation === undefined) return;
  into.files += evaluation.metrics.files;
  const profile = evaluation.profile;
  if (profile === undefined) throw new Error("an apply reported no profile: RERAIL_CODE_RULE_PROFILE is not set");
  into.evaluatedFiles += profile.evaluatedFiles;
  into.evaluatedClauses += profile.evaluatedClauses;
  into.restoredLocalResults += profile.restoredLocalResults;
  into.storageReadNanoseconds += profile.storageReadNanoseconds;
  into.storageManifestNanoseconds += profile.storageManifestNanoseconds;
  into.storageWriteNanoseconds += profile.storageWriteNanoseconds;
}

interface SourceFile {
  path: string;
  absolute: string;
  language: string;
  sourceBytes: number;
}

/** The source files the workspace's rules read, walked as the enforcer walks a repository without a content store. */
async function sourceFiles(workspace: NativeCodeRuleDslWorkspace, root: string): Promise<SourceFile[]> {
  const files: SourceFile[] = [];
  const pending = [root];
  while (pending.length > 0) {
    const directory = pending.pop()!;
    for await (const entry of await opendir(directory)) {
      const absolute = join(directory, entry.name);
      if (entry.isDirectory()) {
        if (!IGNORED_DIRECTORIES.has(entry.name)) pending.push(absolute);
        continue;
      }
      if (!entry.isFile()) continue;
      const path = relative(root, absolute).split(sep).join("/");
      const language = languageFor(path);
      if (language === undefined || !workspace.acceptsPath(path)) continue;
      files.push({ path, absolute, language, sourceBytes: (await stat(absolute)).size });
    }
  }
  if (files.length === 0) throw new Error(`No source file under ${root} is in the rules' scope`);
  return files;
}

/** Edits go to the Python files at the minimum, median, p95 and maximum size. */
function editTargets(files: readonly SourceFile[]): Array<SourceFile & { bucket: string }> {
  const python = files.filter(file => file.path.endsWith(".py"));
  if (python.length === 0) throw new Error("No Python file is in the rules' scope");
  python.sort((left, right) => left.sourceBytes - right.sourceBytes || (left.path < right.path ? -1 : 1));
  const at = (fraction: number) => python[Math.floor((python.length - 1) * fraction)]!;
  return [
    { bucket: "minimum", ...at(0) },
    { bucket: "p50", ...at(0.5) },
    { bucket: "p95", ...at(0.95) },
    { bucket: "maximum", ...at(1) },
  ];
}

function percentile(values: readonly number[], fraction: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)] ?? Number.NaN;
}

function cpuMs(since: NodeJS.CpuUsage): number {
  const used = process.cpuUsage(since);
  return (used.user + used.system) / 1000;
}

function countCodes(failures: ReadonlyArray<{ code: string }>, into: Record<string, number>): void {
  for (const { code } of failures) into[code] = (into[code] ?? 0) + 1;
}

function memorySnapshot(workspace: NativeCodeRuleDslWorkspace) {
  const node = process.memoryUsage();
  return {
    rss: node.rss,
    heapUsed: node.heapUsed,
    external: node.external,
    arrayBuffers: node.arrayBuffers,
    native: JSON.parse(loadNativeBinding().nativeMemoryProfile()) as unknown,
    workspace: JSON.parse(workspace.memoryProfile()) as unknown,
  };
}

interface RunOptions {
  engine: "native" | "sql";
  sqlBatch: number | undefined;
  /** Whether the scan is the stored repository index (`on`) or the enforcer's walk without a content store (`off`). */
  store: boolean;
  /**
   * The one content store every workspace of the run attaches (`--store-path`), created if missing and kept, so a
   * later run, or a later workspace of this one, reuses what earlier ones stored; else each workspace has its own
   * store in a temporary directory.
   */
  storePath: string | undefined;
  /** After the scan, the unchanged re-apply (`--reapply`): every scanned file applied again as it is, then the final apply. */
  reapply: boolean;
  /** The percentage of the workspace's files in scope the walk keeps (`inSample`); 100 keeps every file. */
  sample: number;
  edits: number;
  idleMs: number;
}

/** Whether `path` is in a `percent` sample: the first four bytes of its sha256, mod 100, below `percent`. */
function inSample(path: string, percent: number): boolean {
  return percent >= 100 || createHash("sha256").update(path).digest().readUInt32BE(0) % 100 < percent;
}

/**
 * Every result page of the workspace's current revision: the violations and failures counted, and a sha256 of each
 * list as sorted JSON lines, so two runs' findings compare by digest.
 */
async function readResults(workspace: NativeCodeRuleDslWorkspace) {
  const violations: string[] = [];
  const failures: string[] = [];
  const failureCodes: Record<string, number> = {};
  let cursor = 0;
  let pages = 0;
  while (true) {
    const page = pageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
    pages += 1;
    for (const violation of page.violations) violations.push(JSON.stringify(violation));
    for (const failure of page.failures) failures.push(JSON.stringify(failure));
    countCodes(page.failures, failureCodes);
    if (page.done) break;
    if (page.cursor <= cursor) throw new Error("The result stream did not advance");
    cursor = page.cursor;
  }
  const digest = (lines: string[]) => `sha256:${createHash("sha256").update(lines.sort().join("\n")).digest("hex")}`;
  return { pages, violations: violations.length, failureCodes, violationsSha256: digest(violations), failuresSha256: digest(failures) };
}

/**
 * The enforcer's walk (`#walkAuditSources`): batches of 32 files, closed at 1 MiB, each applied with the
 * workspace-wide pass deferred to the final apply. Returns the last revision applied and the batches.
 */
async function walkApply(workspace: NativeCodeRuleDslWorkspace, files: readonly SourceFile[], revision: number, reuse: Reuse) {
  const binding = loadNativeBinding();
  let batch: Array<{ path: string; language: string; source: string; module?: string | string[] }> = [];
  let bytes = 0;
  let chunks = 0;
  for (const [index, file] of files.entries()) {
    const source = await readFile(file.absolute, "utf8");
    const module = binding.modulePath(file.language, file.absolute);
    batch.push({ path: file.path, language: file.language, source, ...(module === null ? {} : { module }) });
    bytes += Buffer.byteLength(source);
    if (batch.length < AUDIT_BATCH_FILES && bytes < AUDIT_CHUNK_BYTES && index < files.length - 1) continue;
    revision += 1;
    const applied = applySchema.parse(JSON.parse(await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision, files: batch, removed: [], deferGlobal: true }))));
    addReuse(reuse, applied.evaluation);
    chunks += 1;
    batch = [];
    bytes = 0;
  }
  return { revision, chunks };
}

/**
 * The final quiet apply (the deferred workspace-wide pass), the stored revision's record, and every result page, with
 * the time of each.
 */
async function finishScan(workspace: NativeCodeRuleDslWorkspace, root: string, revision: number, store: boolean, reuse: Reuse) {
  const started = performance.now();
  const final = await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision, files: [], removed: [], quiet: true })).catch((error: unknown) => {
    throw new Error(`${root}: the full scan's final apply failed`, { cause: error });
  });
  addReuse(reuse, applySchema.parse(JSON.parse(final)).evaluation);
  const applied = performance.now();
  if (store)
    await workspace.completeStorageRevisionAsync().catch((error: unknown) => {
      throw new Error(`${root}: recording the scanned revision failed`, { cause: error });
    });
  const completed = performance.now();
  const results = await readResults(workspace);
  return { ...results, timings: { finalApplyMs: applied - started, completionMs: completed - applied, resultPagesMs: performance.now() - completed } };
}

async function measureWorkspace(
  name: string,
  root: string,
  rules: LoadedRules,
  options: RunOptions,
  sampler: RssSampler,
) {
  const binding = loadNativeBinding();
  const stores = options.storePath === undefined ? mkdtempSync(join(tmpdir(), "rerail-workspaces-store-")) : undefined;
  const storePath = options.storePath ?? join(stores!, "content.sqlite");
  const phases: Record<string, number> = {};
  sampler.phase();
  const workspaceStarted = performance.now();
  const workspace = new binding.CodeRuleDslWorkspace({
    engine: options.engine,
    ...(options.sqlBatch === undefined ? {} : { sqlBatchSize: options.sqlBatch }),
  });
  try {
    // Registration compiles the SQL program on SQL (`workspace/sql.rs` `register`).
    for (const compiled of rules.compiled) workspace.addCodeRule(compiled);
    const registerMs = performance.now() - workspaceStarted;
    phases.register = sampler.phase();
    const afterRegister = memorySnapshot(workspace);

    // The walk for the files in scope (the scan without a store reads them; the edits pick from them) is not timed.
    const files = (await sourceFiles(workspace, root)).filter(file => inSample(file.path, options.sample));
    const scanCpu = process.cpuUsage();
    const scanStarted = performance.now();
    let revision = 0;
    let discovered = 0;
    let read = 0;
    let restoredFiles = 0;
    let reusableDirectories = 0;
    let chunks = 0;
    let attachMs = 0;
    const reuse = emptyReuse();
    const failureCodes: Record<string, number> = {};
    if (options.store) {
      const attachment = await workspace.enableStorageAsync(root, storePath);
      if (!attachment.attached) throw new Error(`${root}: the content store did not attach: ${attachment.reason ?? "(no reason)"}`);
      attachMs = attachment.discoveryMilliseconds + attachment.publicationMilliseconds;
      while (true) {
        const response = await workspace.indexRepositoryAsync({ ...REPOSITORY_INDEX_OPTIONS, nestedCheckouts: [] }).catch((error: unknown) => {
          throw new Error(`${root}: repository index chunk ${chunks + 1} (${read} files read before it) failed`, { cause: error });
        });
        const chunk = chunkSchema.parse(JSON.parse(response));
        chunks += 1;
        revision = chunk.revision;
        // The index's counts are its totals so far.
        discovered = chunk.discoveredFiles;
        read = chunk.readFiles;
        restoredFiles = chunk.restoredFiles;
        reusableDirectories = chunk.reusableDirectories;
        addReuse(reuse, chunk.evaluation);
        countCodes(chunk.failures, failureCodes);
        if (chunk.overBudget) throw new Error(`${root}: the repository index went over budget`);
        if (chunk.done) break;
      }
    } else {
      discovered = files.length;
      ({ revision, chunks } = await walkApply(workspace, files, revision, reuse));
      read = files.length;
    }
    const indexMs = performance.now() - scanStarted;
    revision += 1;
    const results = await finishScan(workspace, root, revision, options.store, reuse);
    for (const [code, n] of Object.entries(results.failureCodes)) failureCodes[code] = (failureCodes[code] ?? 0) + n;
    const fullScan = {
      wallMs: performance.now() - scanStarted,
      cpuMs: cpuMs(scanCpu),
      attachMs,
      // The attach and index (or the walk's applies), then the final apply, the completion record and the result pages.
      phasesMs: { index: indexMs, ...results.timings },
      chunks,
      discoveredFiles: discovered,
      readFiles: read,
      restoredFiles,
      reusableDirectories,
      reuse,
      pages: results.pages,
      violations: results.violations,
      failureCodes,
      violationsSha256: results.violationsSha256,
      failuresSha256: results.failuresSha256,
    };
    phases.scan = sampler.phase();
    const afterScan = memorySnapshot(workspace);
    const engineReport = JSON.parse(workspace.engineReport()) as unknown;

    // The unchanged re-apply: every scanned file again, as it is, to the workspace that holds them.
    let reapply: { wallMs: number; cpuMs: number; chunks: number; reuse: Reuse; violations: number; violationsSha256: string; failuresSha256: string } | undefined;
    if (options.reapply) {
      const reapplyCpu = process.cpuUsage();
      const reapplyStarted = performance.now();
      const reapplied = emptyReuse();
      const walked = await walkApply(workspace, files, revision, reapplied);
      revision = walked.revision + 1;
      const again = await finishScan(workspace, root, revision, options.store, reapplied);
      reapply = {
        wallMs: performance.now() - reapplyStarted,
        cpuMs: cpuMs(reapplyCpu),
        chunks: walked.chunks,
        reuse: reapplied,
        violations: again.violations,
        violationsSha256: again.violationsSha256,
        failuresSha256: again.failuresSha256,
      };
      phases.reapply = sampler.phase();
    }

    const targets = editTargets(files);
    const edits: Array<{ bucket: string; path: string; sourceBytes: number; inline: boolean; wallMs: number; cpuMs: number }> = [];
    for (let iteration = 0; iteration < options.edits; iteration += 1) {
      const target = targets[iteration % targets.length]!;
      const source = `${await readFile(target.absolute, "utf8")}\n# rerail benchmark edit ${iteration}\n`;
      const module = binding.modulePath(target.language, target.absolute);
      const edited = [{ path: target.path, language: target.language, source, ...(module === null ? {} : { module }) }];
      revision += 1;
      const editCpu = process.cpuUsage();
      const editStarted = performance.now();
      // The enforcer's edit (`#applyRequest`): inline below 4 KiB when the workspace is free, else off the JS thread.
      const inline = source.length < INLINE_EDIT_CHARACTERS ? workspace.tryApply(JSON.stringify({ revision, files: edited, removed: [] })) : null;
      const response = inline ?? (await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision, files: edited, removed: [] })));
      const wallMs = performance.now() - editStarted;
      const cpu = cpuMs(editCpu);
      applySchema.parse(JSON.parse(response));
      workspace.evictInactive();
      edits.push({ bucket: target.bucket, path: target.path, sourceBytes: target.sourceBytes, inline: inline !== null, wallMs, cpuMs: cpu });
    }
    phases.edits = sampler.phase();
    const beforeEvict = memorySnapshot(workspace);
    // The findings after the edits (untimed): each edited file's own, every other file's from the scan.
    const afterEdits = options.edits > 0 ? await readResults(workspace) : undefined;
    workspace.evictInactive();
    await sleep(options.idleMs);
    const afterEvictIdle = memorySnapshot(workspace);
    phases.idle = sampler.phase();
    const wall = edits.map(edit => edit.wallMs);
    const cpu = edits.map(edit => edit.cpuMs);
    return {
      name,
      root,
      commit: git(root, "rev-parse", "HEAD"),
      registerMs,
      fullScan,
      ...(reapply === undefined ? {} : { reapply }),
      edits: {
        count: edits.length,
        wallP50Ms: percentile(wall, 0.5),
        wallP95Ms: percentile(wall, 0.95),
        cpuP50Ms: percentile(cpu, 0.5),
        cpuP95Ms: percentile(cpu, 0.95),
        samples: edits,
        ...(afterEdits === undefined
          ? {}
          : { violations: afterEdits.violations, failureCodes: afterEdits.failureCodes, violationsSha256: afterEdits.violationsSha256, failuresSha256: afterEdits.failuresSha256 }),
      },
      rss: {
        peak: Math.max(...Object.values(phases)),
        peakByPhase: phases,
        afterScan: afterScan.rss,
        final: beforeEvict.rss,
        afterEvictIdle: afterEvictIdle.rss,
      },
      memory: { afterRegister, afterScan, beforeEvict, afterEvictIdle },
      engineReport,
    };
  } finally {
    workspace.close();
    if (stores !== undefined) rmSync(stores, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// Commands

function option(values: Map<string, string[]>, key: string): string | undefined {
  return values.get(key)?.at(-1);
}

function parseArguments(argv: readonly string[], valued: readonly string[]): { values: Map<string, string[]>; positional: string[] } {
  const values = new Map<string, string[]>();
  const positional: string[] = [];
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]!;
    if (!argument.startsWith("--")) {
      positional.push(argument);
      continue;
    }
    if (!valued.includes(argument)) throw new UsageError(`unknown option ${argument}`);
    const value = argv[++index];
    if (value === undefined) throw new UsageError(`${argument} needs a value`);
    values.set(argument, [...(values.get(argument) ?? []), value]);
  }
  return { values, positional };
}

function count(name: string, value: string | undefined, fallback: number, minimum: number): number {
  if (value === undefined) return fallback;
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < minimum) throw new UsageError(`${name} takes an integer ≥ ${minimum}, not ${value}`);
  return parsed;
}

function workspaceRoots(directory: string, only: readonly string[]): Array<{ name: string; root: string }> {
  const missing = only.filter(name => !SUBSET.some(workspace => workspace.name === name));
  if (missing.length > 0) throw new UsageError(`--only names no subset workspace: ${missing.join(", ")}`);
  return SUBSET
    .filter(workspace => only.length === 0 || only.includes(workspace.name))
    .map(({ name, commit }) => {
      const root = resolve(directory, name);
      if (!existsSync(root)) throw new UsageError(`${root} does not exist: run setup first`);
      const head = git(root, "rev-parse", "HEAD");
      if (head !== commit) throw new UsageError(`${root} is at ${head}, not the subset's ${commit}`);
      if (git(root, "status", "--porcelain", "--untracked-files=no") !== "") throw new UsageError(`${root} has local changes`);
      return { name, root: realpathSync(root) };
    });
}

async function run(argv: readonly string[]): Promise<void> {
  const { values } = parseArguments(argv, ["--engine", "--rules", "--workspaces", "--output", "--addon", "--sql-batch", "--store", "--store-path", "--passes", "--reapply", "--sample", "--edits", "--idle-ms", "--only"]);
  const engine = option(values, "--engine");
  if (engine !== "native" && engine !== "sql") throw new UsageError("--engine takes native or sql");
  const ruleDirectories = (values.get("--rules") ?? []).map(directory => resolve(directory));
  if (ruleDirectories.length === 0) throw new UsageError("--rules <dir> is required");
  const workspacesDirectory = option(values, "--workspaces");
  if (workspacesDirectory === undefined) throw new UsageError("--workspaces <dir> is required");
  const output = option(values, "--output");
  if (output === undefined) throw new UsageError("--output <file.json> is required");
  const sqlBatch = option(values, "--sql-batch");
  if (sqlBatch !== undefined && engine !== "sql") throw new UsageError("--sql-batch needs --engine sql");
  const store = option(values, "--store") ?? "on";
  if (store !== "on" && store !== "off") throw new UsageError("--store takes on or off");
  const sample = count("--sample", option(values, "--sample"), 100, 1);
  // The stored index reads the whole checkout; only the walk without a store can keep a sample of it.
  if (sample < 100 && store === "on") throw new UsageError("--sample needs --store off");
  const storePath = option(values, "--store-path");
  if (storePath !== undefined && store !== "on") throw new UsageError("--store-path needs --store on");
  const reapply = option(values, "--reapply") ?? "off";
  if (reapply !== "on" && reapply !== "off") throw new UsageError("--reapply takes on or off");
  const passes = count("--passes", option(values, "--passes"), 1, 1);
  const options: RunOptions = {
    engine,
    sqlBatch: sqlBatch === undefined ? undefined : count("--sql-batch", sqlBatch, 0, 1),
    store: store === "on",
    storePath: storePath === undefined ? undefined : resolve(storePath),
    reapply: reapply === "on",
    sample,
    edits: count("--edits", option(values, "--edits"), 20, 0),
    idleMs: count("--idle-ms", option(values, "--idle-ms"), 5_000, 0),
  };
  // The reuse counters (files evaluated, clause results restored) are the profile's.
  process.env.RERAIL_CODE_RULE_PROFILE = "1";
  const addon = option(values, "--addon");
  if (addon !== undefined) process.env.RERAIL_ENGINE_PATH = resolve(addon);
  const addonPath = realpathSync(resolveNativeBindingPath());
  process.env.RERAIL_ENGINE_PATH = addonPath;
  const roots = workspaceRoots(workspacesDirectory, (values.get("--only") ?? []).flatMap(names => names.split(",")));

  const sampler = new RssSampler();
  const started = performance.now();
  const loadStart = loadavg();
  const rssStart = process.memoryUsage.rss();
  const binding = loadNativeBinding();
  const kernelVersion = binding.health().kernelVersion;
  const rules = loadRules(ruleDirectories);
  const compileMs = performance.now() - started;
  const peakCompile = sampler.phase();
  const rssAfterCompile = process.memoryUsage.rss();
  const nativeAfterCompile = JSON.parse(binding.nativeMemoryProfile()) as unknown;
  process.stderr.write(`rerail-workspaces: ${engine}, ${rules.compiled.length} rules (${rules.rejected.length} rejected), compiled in ${Math.round(compileMs)} ms, RSS ${(rssAfterCompile / MIB).toFixed(0)} MiB\n`);
  const workspaces = [];
  for (let pass = 1; pass <= passes; pass += 1)
    for (const { name, root } of roots) {
      const measured = { pass, ...(await measureWorkspace(name, root, rules, options, sampler)) };
      workspaces.push(measured);
      const { reuse } = measured.fullScan;
      process.stderr.write(
        `rerail-workspaces: ${name} (pass ${pass}): scan ${Math.round(measured.fullScan.wallMs)} ms, ${measured.fullScan.discoveredFiles} files: ` +
          `${measured.fullScan.restoredFiles} restored by the index, ${reuse.files} applied, ${reuse.evaluatedFiles} evaluated (${reuse.evaluatedClauses} clauses, ${reuse.restoredLocalResults} results restored); ` +
          (measured.reapply === undefined ? "" : `re-apply ${Math.round(measured.reapply.wallMs)} ms, ${measured.reapply.reuse.evaluatedFiles} evaluated; `) +
          `edits p50 ${measured.edits.wallP50Ms.toFixed(1)} / p95 ${measured.edits.wallP95Ms.toFixed(1)} ms, ` +
          `peak ${(measured.rss.peak / MIB).toFixed(0)} MiB, final ${(measured.rss.final / MIB).toFixed(0)} MiB, after evict+idle ${(measured.rss.afterEvictIdle / MIB).toFixed(0)} MiB, ` +
          `${measured.fullScan.violations} violations, ${JSON.stringify(measured.fullScan.failureCodes)}\n`,
      );
    }
  const result = {
    benchmark: "rerail-workspaces/v1",
    engine,
    sqlBatch: options.sqlBatch,
    store,
    storePath: options.storePath,
    passes,
    reapply: options.reapply,
    sample: options.sample,
    addon: { path: addonPath, sha256: `sha256:${createHash("sha256").update(readFileSync(addonPath)).digest("hex")}`, kernelVersion },
    rules: { directories: ruleDirectories, count: rules.compiled.length, sha256: rules.sha256, rejected: rules.rejected },
    edits: options.edits,
    idleMs: options.idleMs,
    machine: { cpus: cpus().length, model: cpus()[0]?.model, os: `${process.platform} ${release()}`, node: process.version },
    loadAverage: { start: loadStart, end: loadavg() },
    wallMs: performance.now() - started,
    rss: {
      start: rssStart,
      afterCompile: rssAfterCompile,
      nativeAfterCompile,
      peakCompile,
      peakSampled: sampler.peak,
      // The kernel's resident-set high-water mark (`getrusage`, KiB): no sampling gap, never reset.
      maxRss: process.resourceUsage().maxRSS * 1024,
      end: process.memoryUsage.rss(),
    },
    workspaces,
  };
  await sampler.stop();
  writeFileSync(resolve(output), `${JSON.stringify(result, null, 2)}\n`);
  process.stderr.write(
    `rerail-workspaces: ${engine}: ${workspaces.length} workspaces in ${(result.wallMs / 1000).toFixed(0)} s, peak RSS ${(result.rss.peakSampled / MIB).toFixed(0)} MiB sampled, ${(result.rss.maxRss / MIB).toFixed(0)} MiB getrusage; ${output}\n`,
  );
}

function setup(argv: readonly string[]): void {
  const { values } = parseArguments(argv, ["--data-layer", "--workspaces"]);
  const clone = option(values, "--data-layer");
  const directory = option(values, "--workspaces");
  if (clone === undefined || directory === undefined) throw new UsageError("setup needs --data-layer <clone> --workspaces <dir>");
  for (const { name, commit } of SUBSET) {
    const root = resolve(directory, name);
    if (existsSync(root)) {
      const head = git(root, "rev-parse", "HEAD");
      if (head !== commit) throw new UsageError(`${root} exists at ${head}, not ${commit}`);
      process.stdout.write(`${name}: ${root} at ${commit}\n`);
      continue;
    }
    git(resolve(clone), "worktree", "add", "--detach", root, commit);
    process.stdout.write(`${name}: added ${root} at ${commit}\n`);
  }
}

type RunResult = {
  engine: string;
  rules: { count: number };
  wallMs: number;
  rss: { afterCompile: number; peakCompile: number; peakSampled: number; maxRss: number; end: number };
  workspaces: Array<{
    name: string;
    fullScan: { wallMs: number; cpuMs: number; violations: number; failureCodes: Record<string, number>; violationsSha256?: string; failuresSha256?: string };
    edits: { wallP50Ms: number; wallP95Ms: number; samples: Array<{ wallMs: number }>; violationsSha256?: string; failuresSha256?: string };
    rss: { peak: number; final: number; afterEvictIdle: number };
  }>;
};

/** Prints two runs side by side; returns whether their findings (violations, after the scan and after the edits) are equal. */
function compare(argv: readonly string[]): boolean {
  const { positional } = parseArguments(argv, []);
  if (positional.length !== 2) throw new UsageError("compare takes two run outputs: <native.json> <sql.json>");
  const [left, right] = positional.map(path => JSON.parse(readFileSync(path, "utf8")) as RunResult) as [RunResult, RunResult];
  const mib = (bytes: number) => `${(bytes / MIB).toFixed(0)} MiB`;
  const ms = (value: number) => `${value.toFixed(value < 100 ? 1 : 0)} ms`;
  const ratio = (a: number, b: number) => (a > 0 ? `${(b / a).toFixed(2)}×` : "-");
  const rows: Array<[string, number, number, (value: number) => string]> = [
    ["peak RSS, sampled (run)", left.rss.peakSampled, right.rss.peakSampled, mib],
    ["peak RSS, getrusage (run)", left.rss.maxRss, right.rss.maxRss, mib],
    ["RSS after rule compile", left.rss.afterCompile, right.rss.afterCompile, mib],
    ["RSS at the end of the run", left.rss.end, right.rss.end, mib],
    ["run wall time", left.wallMs, right.wallMs, value => `${(value / 1000).toFixed(1)} s`],
  ];
  const scans = (result: RunResult) => result.workspaces.reduce((sum, workspace) => sum + workspace.fullScan.wallMs, 0);
  rows.push(["full scan wall (all workspaces)", scans(left), scans(right), value => `${(value / 1000).toFixed(1)} s`]);
  const all = (result: RunResult) => result.workspaces.flatMap(workspace => workspace.edits.samples.map(sample => sample.wallMs));
  rows.push(["edit p50 (all workspaces)", percentile(all(left), 0.5), percentile(all(right), 0.5), ms]);
  rows.push(["edit p95 (all workspaces)", percentile(all(left), 0.95), percentile(all(right), 0.95), ms]);
  const lines = [`| metric | ${left.engine} | ${right.engine} | ${right.engine}/${left.engine} |`, "| --- | ---: | ---: | ---: |"];
  for (const [label, a, b, format] of rows) lines.push(`| ${label} | ${format(a)} | ${format(b)} | ${ratio(a, b)} |`);
  // Per workspace, one row: each cell is `left / right`.
  const pair = (a: number, b: number, format: (value: number) => string) => `${format(a)} / ${format(b)}`;
  const codes = (result: Record<string, number>) => Object.entries(result).sort().map(([code, n]) => `${code} ${n}`).join(", ") || "none";
  lines.push(
    "",
    `Per workspace, ${left.engine} / ${right.engine}:`,
    "",
    "| workspace | full scan wall | full scan CPU | edit p50 | edit p95 | peak RSS | final RSS | RSS after evict + idle | violations | failures |",
    "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |",
  );
  // Findings: violations must be equal; failures may differ where native's work budget fails a file (the design's
  // §Budgets), so they are reported only.
  const findings: string[] = [];
  let equal = true;
  const same = (a: string | undefined, b: string | undefined) => (a === undefined || b === undefined ? "not recorded" : a === b ? "equal" : "DIFFER");
  for (const [index, a] of left.workspaces.entries()) {
    const b = right.workspaces[index];
    if (b === undefined || b.name !== a.name) throw new UsageError(`the runs' workspaces differ at ${a.name}`);
    const seconds = (value: number) => `${(value / 1000).toFixed(1)} s`;
    lines.push(
      `| ${a.name} | ${pair(a.fullScan.wallMs, b.fullScan.wallMs, seconds)} | ${pair(a.fullScan.cpuMs, b.fullScan.cpuMs, seconds)} | ` +
        `${pair(a.edits.wallP50Ms, b.edits.wallP50Ms, ms)} | ${pair(a.edits.wallP95Ms, b.edits.wallP95Ms, ms)} | ` +
        `${pair(a.rss.peak, b.rss.peak, mib)} | ${pair(a.rss.final, b.rss.final, mib)} | ${pair(a.rss.afterEvictIdle, b.rss.afterEvictIdle, mib)} | ` +
        `${a.fullScan.violations} / ${b.fullScan.violations} | ${codes(a.fullScan.failureCodes)} / ${codes(b.fullScan.failureCodes)} |`,
    );
    const scan = same(a.fullScan.violationsSha256, b.fullScan.violationsSha256);
    const edited = same(a.edits.violationsSha256, b.edits.violationsSha256);
    equal &&= scan !== "DIFFER" && edited !== "DIFFER" && a.fullScan.violations === b.fullScan.violations;
    findings.push(
      `- ${a.name}: violations after the scan ${scan}, after the edits ${edited}; ` +
        `failures after the scan ${same(a.fullScan.failuresSha256, b.fullScan.failuresSha256)}, after the edits ${same(a.edits.failuresSha256, b.edits.failuresSha256)}`,
    );
  }
  lines.push("", `Findings, ${left.engine} against ${right.engine}:`, "", ...findings);
  process.stdout.write(`${lines.join("\n")}\n`);
  return equal;
}

/**
 * The production-path loop: `run` on native, then on SQL, each in its own process (each peak is its own), over the
 * fixed production-shape workload (`PROD_PATH`), then `compare`. Fails where the findings differ.
 */
function prodPath(argv: readonly string[]): void {
  const { values } = parseArguments(argv, ["--rules", "--workspaces", "--output-dir", "--addon", "--only", "--sample", "--edits"]);
  const outputDirectory = option(values, "--output-dir");
  if (outputDirectory === undefined) throw new UsageError("--output-dir <dir> is required");
  mkdirSync(outputDirectory, { recursive: true });
  const shared = [
    ...(values.get("--rules") ?? []).flatMap(directory => ["--rules", directory]),
    "--workspaces", option(values, "--workspaces") ?? "",
    ...(option(values, "--addon") === undefined ? [] : ["--addon", option(values, "--addon")!]),
    "--store", "off",
    "--only", option(values, "--only") ?? PROD_PATH.only,
    "--sample", option(values, "--sample") ?? String(PROD_PATH.sample),
    "--edits", option(values, "--edits") ?? String(PROD_PATH.edits),
    "--idle-ms", "0",
  ];
  const outputs = (["native", "sql"] as const).map(engine => {
    const output = resolve(outputDirectory, `${engine}.json`);
    const child = spawnSync(process.execPath, [...process.execArgv, fileURLToPath(import.meta.url), "run", "--engine", engine, ...shared, "--output", output], { stdio: "inherit" });
    if (child.status !== 0) throw new Error(`rerail-workspaces: the ${engine} run failed (${child.status ?? child.signal})`);
    return output;
  });
  if (!compare(outputs)) {
    process.stderr.write("rerail-workspaces: the engines' findings differ\n");
    process.exitCode = 1;
  }
}

const [command, ...rest] = process.argv.slice(2);
try {
  if (command === "run") await run(rest);
  else if (command === "setup") setup(rest);
  else if (command === "compare") process.exitCode = compare(rest) ? 0 : 1;
  else if (command === "prod-path") prodPath(rest);
  else throw new UsageError(`unknown command ${command ?? "(none)"}: setup | run | compare | prod-path`);
} catch (error) {
  if (!(error instanceof UsageError)) throw error;
  process.stderr.write(`rerail-workspaces: ${error.message}\n`);
  process.exitCode = 2;
}
