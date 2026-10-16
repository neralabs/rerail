// `whole-tree`: one apply of every ported rule over every Python file of the corpus, in a fresh process per run.
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { cpus, loadavg, totalmem } from "node:os";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { z } from "zod";
import { blindFileLine, compareStrings, failureLine, RuleWorkspace, sha256, violationLine, type Engine, type PortedRule } from "./engine.js";
import type { Repository } from "./corpus.js";

const runSchema = z.object({
  run: z.number().int(),
  applyWallMs: z.number(),
  applyCpuMs: z.number(),
  processWallMs: z.number(),
  processCpuMs: z.number(),
  files: z.number().int(),
  findings: z.number().int(),
  failures: z.number().int(),
  blindFiles: z.number().int(),
  digest: z.string(),
  blindDigest: z.string(),
  status: z.string(),
  failureCodes: z.record(z.string(), z.number()),
  loadAverage: z.array(z.number()),
  lines: z.string(),
});
export type WholeTreeRun = z.infer<typeof runSchema>;

/**
 * One run in this process: every accepted file in one `applyAsync`. Writes the sorted violation and failure lines to
 * `whole-tree-<run>.lines.txt` (their sha256 is the digest) and the blind files to `whole-tree-<run>.blind.txt`.
 */
export async function runWholeTree(options: { engine: Engine; repository: Repository; rules: PortedRule[]; output: string; run: number; processStarted: number; processCpu: NodeJS.CpuUsage }): Promise<WholeTreeRun> {
  const { engine, repository, rules, output, run } = options;
  const workspace = new RuleWorkspace(engine, rules);
  try {
    const accepted = repository.files.filter(file => workspace.accepts(file.path));
    const loadAverage = loadavg();
    const { wallMs, cpuMs, result } = await workspace.apply(accepted);
    const lines = [...result.violations.map(violationLine), ...result.failures.map(failureLine)].sort(compareStrings);
    const blind = result.blindFiles.map(blindFileLine).sort(compareStrings);
    const text = lines.map(line => `${line}\n`).join("");
    const blindText = blind.map(line => `${line}\n`).join("");
    mkdirSync(output, { recursive: true });
    const linesPath = resolve(output, `whole-tree-${run}.lines.txt`);
    writeFileSync(linesPath, text);
    writeFileSync(resolve(output, `whole-tree-${run}.blind.txt`), blindText);
    // Failures grouped by code and message, numbers (lines, columns, counts) folded so one group spans many files.
    const failureCodes: Record<string, number> = {};
    for (const failure of result.failures) {
      const group = `${failure.code}: ${failure.message.replace(/\d+/g, "N")}`;
      failureCodes[group] = (failureCodes[group] ?? 0) + 1;
    }
    const processCpu = process.cpuUsage(options.processCpu);
    return {
      run,
      applyWallMs: wallMs,
      applyCpuMs: cpuMs,
      processWallMs: performance.now() - options.processStarted,
      processCpuMs: (processCpu.user + processCpu.system) / 1000,
      files: accepted.length,
      findings: result.violations.length,
      failures: result.failures.length,
      blindFiles: result.blindFiles.length,
      digest: sha256(text),
      blindDigest: sha256(blindText),
      status: result.status,
      failureCodes,
      loadAverage,
      lines: linesPath,
    };
  } finally {
    workspace.close();
  }
}

/** Runs `runs` whole-tree evaluations, each in a child process of its own, one after another. */
export async function runWholeTreeChildren(options: { script: string; childArguments: string[]; runs: number }): Promise<WholeTreeRun[]> {
  const results: WholeTreeRun[] = [];
  for (let run = 1; run <= options.runs; run += 1) {
    const stdout = await new Promise<string>((resolveRun, reject) => {
      const child = spawn(process.execPath, [...process.execArgv, options.script, "whole-tree", ...options.childArguments, "--child-run", String(run)], { stdio: ["ignore", "pipe", "inherit"] });
      const chunks: Buffer[] = [];
      child.stdout.on("data", chunk => chunks.push(chunk));
      child.on("error", reject);
      child.on("close", code => (code === 0 ? resolveRun(Buffer.concat(chunks).toString("utf8")) : reject(new Error(`whole-tree run ${run} exited ${code}`))));
    });
    const result = runSchema.parse(JSON.parse(stdout));
    process.stderr.write(`run ${run}: ${formatRun(result)}\n`);
    results.push(result);
  }
  return results;
}

export function formatRun(run: WholeTreeRun): string {
  return `apply wall ${(run.applyWallMs / 1000).toFixed(2)} s, apply CPU ${(run.applyCpuMs / 1000).toFixed(2)} s (process wall ${(run.processWallMs / 1000).toFixed(2)} s, CPU ${(run.processCpuMs / 1000).toFixed(2)} s); ${run.files} files, ${run.findings} findings, ${run.failures} failures, ${run.blindFiles} blind files; ${run.digest}; load ${run.loadAverage.map(value => value.toFixed(2)).join(" ")}`;
}

/** `whole-tree.json` and the console summary for every run; the exit code is 1 when the runs' digests differ. */
export function reportWholeTree(options: { engine: Engine; repository: Repository; rules: PortedRule[]; output: string; runs: WholeTreeRun[] }): number {
  const { engine, repository, rules, output, runs } = options;
  const digests = new Set(runs.map(run => `${run.digest} ${run.blindDigest}`));
  const report = {
    format: "rerail.data-layer-whole-tree/v1",
    addon: engine.addon,
    workerThreads: engine.workerThreads,
    capabilities: { path: engine.capabilities.path, sha256: engine.capabilities.sha256 },
    corpus: { root: repository.root, commit: repository.git.commit, changes: repository.git.changes, files: repository.files.length, bytes: repository.bytes, skipped: repository.skipped },
    rules: rules.map(rule => ({ linter: rule.linter, path: rule.path, sha256: rule.sha256, clauses: rule.clauses })),
    machine: { cpus: cpus().length, model: cpus()[0]?.model, memoryBytes: totalmem(), platform: `${process.platform} ${process.arch}`, node: process.version },
    runs,
    stable: digests.size === 1,
  };
  mkdirSync(output, { recursive: true });
  writeFileSync(resolve(output, "whole-tree.json"), `${JSON.stringify(report, null, 2)}\n`);
  console.log(`addon ${engine.addon.path} (${engine.addon.kernelVersion}, work ${engine.addon.work}, ${engine.workerThreads} workers)`);
  console.log(`corpus ${repository.root} @ ${repository.git.commit.slice(0, 12)}; ${rules.length} rules`);
  for (const run of runs) console.log(`run ${run.run}: ${formatRun(run)}`);
  if (runs[0] !== undefined) {
    const failures = Object.entries(runs[0].failureCodes).sort(([left], [right]) => compareStrings(left, right));
    for (const [code, count] of failures) console.log(`  failure ${count}× ${code}`);
  }
  console.log(digests.size === 1 ? `digest stable across ${runs.length} run(s)` : `DIGEST UNSTABLE: ${[...digests].join(" | ")}`);
  console.log(`outputs: ${output}`);
  return digests.size === 1 ? 0 : 1;
}
