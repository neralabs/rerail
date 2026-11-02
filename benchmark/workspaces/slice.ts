// `run --engine slice`: the first slice (docs/final-shape/06-first-slice.md, `cr/slice/napi.rs`) over the memory47
// workspaces, as `run --engine native` drives W over them. Per workspace, a `CodeRuleSlice` over its own content store
// (or `--store-path`'s, shared): the pack's policies lowered, a cold audit, the unchanged apply (it must evaluate
// nothing), then `--edits` edits appending a comment to the Python files at the minimum, median, p95 and maximum size in
// turn, native's targets and text. An edit is an editor buffer over the directory snapshot; the earlier edits' buffers
// stay, as native's workspace keeps them, so each apply changes one document. A phase that throws is recorded with its
// error and ends that workspace (the slice's failures are measurements here). `--findings-out` writes the cold audit's
// and the last edit's findings and coverage for `compare-findings`, and the output names the clauses that lowered.
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { cpus, loadavg, release, tmpdir } from "node:os";
import { basename, dirname, join, relative, sep } from "node:path";
import { performance } from "node:perf_hooks";
import { z } from "zod";
import { IGNORED_DIRECTORIES, loadNativeBinding } from "../../src/engine/index.js";
import type { NativeCodeRuleSlice, NativeSliceBuffer, NativeSlicePolicy } from "../../src/engine/native/native.js";

export interface SliceRunOptions {
  /** The rule directories: every `rerail.policy/v1` document under them, by file stem, as the slice's tests load a pack. */
  rules: readonly string[];
  capabilities: string;
  roots: ReadonlyArray<{ name: string; root: string; commit: string }>;
  backend: "cpu" | "metal";
  /** What the cold audit reads: the directory's snapshot, or its Git checkout's `HEAD` tree. Edits are buffers over the directory. */
  observer: "directory" | "head";
  edits: number;
  /** One content store for every workspace of the run, kept; else each workspace has its own, removed after it. */
  storePath: string | undefined;
  output: string;
  findingsOut: string | undefined;
  addon: { path: string; sha256: string };
}

/** The run's RSS sampler (`RssSampler` in workspaces.ts): `phase` starts a phase and returns the peak of the one it ends. */
export interface PhaseSampler {
  phase(): number;
  readonly peak: number;
}

const RULE_FORMAT = "rerail.policy/v1";
const MIB = 1024 * 1024;

const reportSchema = z.object({
  findings: z.array(z.object({ rule: z.string(), code: z.string(), path: z.string(), line: z.number().int() })),
  coverage: z.array(z.object({ rule: z.string(), path: z.string(), coverage: z.enum(["complete", "unknown"]), reason: z.string().optional() })),
  evaluated: z.array(z.string()),
  reused: z.array(z.string()),
  rerun: z.number().int(),
  changed: z.object({ matches: z.array(z.string()), findings: z.array(z.string()) }),
});
type Report = z.infer<typeof reportSchema>;

function percentile(values: readonly number[], fraction: number): number {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.max(0, Math.ceil(sorted.length * fraction) - 1)] ?? Number.NaN;
}

function ruleFiles(directory: string): string[] {
  return readdirSync(directory, { recursive: true, withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
    .map(entry => join(entry.parentPath, entry.name))
    .sort();
}

/**
 * The policies handed to the slice: every `rerail.policy/v1` document under the rule directories that compiles (native's
 * `loadRules` keeps the same ones), each named by its file stem as the slice's tests name them.
 */
function loadPolicies(directories: readonly string[], capabilities: string) {
  const binding = loadNativeBinding();
  const policies: Array<NativeSlicePolicy & { id: string }> = [];
  const rejected: Array<{ path: string; id: string; reason: string }> = [];
  const hash = createHash("sha256");
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
        binding.CodeRuleDslRuntime.compile(source, "json", capabilities);
      } catch (error) {
        rejected.push({ path, id: document.id, reason: error instanceof Error ? error.message : String(error) });
        continue;
      }
      policies.push({ id: document.id, name: basename(path, ".json"), source, format: "json" });
    }
  if (policies.length === 0) throw new Error(`No rule compiled under ${directories.join(", ")}`);
  return { policies, rejected, sha256: `sha256:${hash.digest("hex")}` };
}

/**
 * The clauses the slice lowers, as (rule id, clause id) with how many of the clause's Python matchers lowered. The napi
 * slice reports only a count, so each policy is lowered alone, and a policy with more than one clause is lowered once
 * per clause with its other clauses left out; the per-clause counts must sum to the policy's.
 */
function loweredClauses(policies: ReadonlyArray<NativeSlicePolicy & { id: string }>, capabilities: string) {
  const binding = loadNativeBinding();
  const directory = mkdtempSync(join(tmpdir(), "rerail-slice-probe-"));
  const store = join(directory, "probe.sqlite");
  const lowered: Array<{ rule: string; clause: string; matchers: number }> = [];
  try {
    for (const policy of policies) {
      const whole = new binding.CodeRuleSlice(store, [policy], capabilities, "cpu").clauses();
      if (whole === 0) continue;
      const document = JSON.parse(policy.source) as { clauses: Array<{ id: string }> };
      let sum = 0;
      for (const clause of document.clauses) {
        const alone = { ...policy, source: JSON.stringify({ ...document, clauses: [clause] }) };
        const matchers = document.clauses.length === 1 ? whole : new binding.CodeRuleSlice(store, [alone], capabilities, "cpu").clauses();
        if (matchers > 0) lowered.push({ rule: policy.id, clause: clause.id, matchers });
        sum += matchers;
      }
      if (sum !== whole) throw new Error(`${policy.id}: its clauses lower ${sum} matchers alone, ${whole} together`);
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
  return lowered;
}

interface PythonFile {
  path: string;
  absolute: string;
  sourceBytes: number;
}

/** The workspace's Python files, walked as native's harness walks it (its ignored directories, `.git` left out). */
function pythonFiles(root: string): PythonFile[] {
  const files: PythonFile[] = [];
  const pending = [root];
  while (pending.length > 0) {
    const directory = pending.pop()!;
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const absolute = join(directory, entry.name);
      if (entry.isDirectory()) {
        if (entry.name !== ".git" && !IGNORED_DIRECTORIES.has(entry.name)) pending.push(absolute);
      } else if (entry.isFile() && entry.name.endsWith(".py")) {
        files.push({ path: relative(root, absolute).split(sep).join("/"), absolute, sourceBytes: statSync(absolute).size });
      }
    }
  }
  return files;
}

/** Native's edit targets: the Python files at the minimum, median, p95 and maximum size, ties by path. */
function editTargets(files: readonly PythonFile[]): Array<PythonFile & { bucket: string }> {
  const python = [...files].sort((left, right) => left.sourceBytes - right.sourceBytes || (left.path < right.path ? -1 : 1));
  if (python.length === 0) throw new Error("No Python file in the workspace");
  const at = (fraction: number) => python[Math.floor((python.length - 1) * fraction)]!;
  return [
    { bucket: "minimum", ...at(0) },
    { bucket: "p50", ...at(0.5) },
    { bucket: "p95", ...at(0.95) },
    { bucket: "maximum", ...at(1) },
  ];
}

function summary(report: Report) {
  return {
    findings: report.findings.length,
    unknown: report.coverage.filter(entry => entry.coverage === "unknown").length,
    evaluated: report.evaluated.length,
    reused: report.reused.length,
    rerun: report.rerun,
    changedMatches: report.changed.matches.length,
    changedFindings: report.changed.findings.length,
  };
}

function writeFindings(directory: string, file: string, report: Report): void {
  const findings = [...report.findings].sort((a, b) => a.path.localeCompare(b.path) || a.line - b.line || a.rule.localeCompare(b.rule) || a.code.localeCompare(b.code));
  const unknown = report.coverage.filter(entry => entry.coverage === "unknown");
  writeFileSync(join(directory, file), `${JSON.stringify({ findings, unknown })}\n`);
}

/** One timed slice call: wall and CPU time (user + system, every thread of the process), then its report parsed. */
function timed(call: () => string) {
  const cpu = process.cpuUsage();
  const started = performance.now();
  const json = call();
  const wallMs = performance.now() - started;
  const used = process.cpuUsage(cpu);
  return { wallMs, cpuMs: (used.user + used.system) / 1000, report: reportSchema.parse(JSON.parse(json)) };
}

async function measureWorkspace(
  name: string,
  root: string,
  policies: readonly NativeSlicePolicy[],
  options: SliceRunOptions,
  sampler: PhaseSampler,
) {
  const binding = loadNativeBinding();
  const stores = options.storePath === undefined ? mkdtempSync(join(tmpdir(), "rerail-slice-store-")) : undefined;
  const storePath = options.storePath ?? join(stores!, "content.sqlite");
  const phases: Record<string, number> = {};
  const memory: Record<string, unknown> = {};
  const snapshot = (phase: string) => {
    memory[phase] = { rss: process.memoryUsage.rss(), native: JSON.parse(binding.nativeMemoryProfile()) as unknown };
  };
  const files = pythonFiles(root);
  const result: Record<string, unknown> = {
    name,
    root,
    pythonFiles: files.length,
    pythonBytes: files.reduce((sum, file) => sum + file.sourceBytes, 0),
  };
  let failed: { phase: string; message: string } | undefined;
  let phase = "lower";
  sampler.phase();
  let slice: NativeCodeRuleSlice | undefined;
  try {
    const lowerStarted = performance.now();
    slice = new binding.CodeRuleSlice(storePath, [...policies], options.capabilities, options.backend);
    result.lowerMs = performance.now() - lowerStarted;
    result.clauses = slice.clauses();
    phases.lower = sampler.phase();
    snapshot("afterLower");

    phase = "cold";
    const cold = timed(() => slice!.audit(name, root, options.observer));
    result.cold = { wallMs: cold.wallMs, cpuMs: cold.cpuMs, ...summary(cold.report) };
    phases.cold = sampler.phase();
    snapshot("afterCold");
    if (options.findingsOut !== undefined) writeFindings(options.findingsOut, `${name}.scan.json`, cold.report);
    process.stderr.write(`rerail-workspaces: slice ${name}: cold ${(cold.wallMs / 1000).toFixed(1)} s wall, ${(cold.cpuMs / 1000).toFixed(1)} s CPU, ${cold.report.findings.length} findings, peak ${(phases.cold / MIB).toFixed(0)} MiB\n`);

    phase = "unchanged";
    const unchanged = timed(() => slice!.apply(name, root, "directory"));
    result.unchanged = { wallMs: unchanged.wallMs, cpuMs: unchanged.cpuMs, ...summary(unchanged.report) };
    phases.unchanged = sampler.phase();

    phase = "edits";
    const targets = editTargets(files);
    const buffers = new Map<string, string>();
    const samples = [];
    let last: Report | undefined;
    for (let iteration = 0; iteration < options.edits; iteration += 1) {
      const target = targets[iteration % targets.length]!;
      buffers.set(target.path, `${readFileSync(target.absolute, "utf8")}\n# rerail benchmark edit ${iteration}\n`);
      const edited: NativeSliceBuffer[] = [...buffers].map(([path, source]) => ({ path, source }));
      const edit = timed(() => slice!.apply(name, root, "directory", edited));
      last = edit.report;
      samples.push({ bucket: target.bucket, path: target.path, sourceBytes: target.sourceBytes, wallMs: edit.wallMs, cpuMs: edit.cpuMs, ...summary(edit.report) });
    }
    phases.edits = sampler.phase();
    snapshot("afterEdits");
    if (options.findingsOut !== undefined && last !== undefined) writeFindings(options.findingsOut, `${name}.edits.json`, last);
    const wall = samples.map(sample => sample.wallMs);
    const cpu = samples.map(sample => sample.cpuMs);
    result.edits = {
      count: samples.length,
      wallP50Ms: percentile(wall, 0.5),
      wallP95Ms: percentile(wall, 0.95),
      cpuP50Ms: percentile(cpu, 0.5),
      cpuP95Ms: percentile(cpu, 0.95),
      samples,
      ...(last === undefined ? {} : { findings: last.findings.length }),
    };
  } catch (error) {
    failed = { phase, message: error instanceof Error ? error.message : String(error) };
    phases[phase] = sampler.phase();
    snapshot(`at ${phase} failure`);
    process.stderr.write(`rerail-workspaces: slice ${name}: ${phase} failed: ${failed.message}\n`);
  } finally {
    slice = undefined;
    // The store's directory on disk: its database, WAL and overlay spill (with `--store-path`, whatever else is there).
    const storeDirectory = stores ?? dirname(storePath);
    const bytes = readdirSync(storeDirectory, { recursive: true, withFileTypes: true })
      .filter(entry => entry.isFile())
      .reduce((sum, entry) => sum + statSync(join(entry.parentPath, entry.name)).size, 0);
    result.store = { path: storePath, bytes };
    if (stores !== undefined) rmSync(stores, { recursive: true, force: true });
  }
  return { ...result, ...(failed === undefined ? {} : { failed }), rss: { peak: Math.max(...Object.values(phases)), peakByPhase: phases }, memory };
}

export async function runSlice(options: SliceRunOptions, sampler: PhaseSampler): Promise<void> {
  const started = performance.now();
  const loadStart = loadavg();
  const rssStart = process.memoryUsage.rss();
  if (options.findingsOut !== undefined) mkdirSync(options.findingsOut, { recursive: true });
  const { policies, rejected, sha256 } = loadPolicies(options.rules, options.capabilities);
  const probeStarted = performance.now();
  const lowered = loweredClauses(policies, options.capabilities);
  const probeMs = performance.now() - probeStarted;
  const peakCompile = sampler.phase();
  const rssAfterCompile = process.memoryUsage.rss();
  process.stderr.write(
    `rerail-workspaces: slice (${options.backend}, ${options.observer}): ${policies.length} policies (${rejected.length} rejected), ` +
      `${lowered.length} clauses lower (${lowered.reduce((sum, clause) => sum + clause.matchers, 0)} matchers), probed in ${Math.round(probeMs)} ms\n`,
  );
  const workspaces = [];
  for (const { name, root, commit } of options.roots) workspaces.push({ commit, ...(await measureWorkspace(name, root, policies, options, sampler)) });
  const result = {
    benchmark: "rerail-workspaces/v1",
    engine: "slice",
    backend: options.backend,
    observer: options.observer,
    edits: options.edits,
    storePath: options.storePath,
    addon: options.addon,
    rules: { directories: options.rules, count: policies.length, sha256, rejected },
    lowered,
    machine: { cpus: cpus().length, model: cpus()[0]?.model, os: `${process.platform} ${release()}`, node: process.version },
    loadAverage: { start: loadStart, end: loadavg() },
    wallMs: performance.now() - started,
    rss: {
      start: rssStart,
      afterCompile: rssAfterCompile,
      peakCompile,
      peakSampled: sampler.peak,
      // The kernel's resident-set high-water mark (`getrusage`, KiB): no sampling gap, never reset.
      maxRss: process.resourceUsage().maxRSS * 1024,
      end: process.memoryUsage.rss(),
    },
    workspaces,
  };
  writeFileSync(options.output, `${JSON.stringify(result, null, 2)}\n`);
  process.stderr.write(
    `rerail-workspaces: slice: ${workspaces.length} workspaces in ${(result.wallMs / 1000).toFixed(0)} s, peak RSS ${(result.rss.peakSampled / MIB).toFixed(0)} MiB sampled, ${(result.rss.maxRss / MIB).toFixed(0)} MiB getrusage; ${options.output}\n`,
  );
}
