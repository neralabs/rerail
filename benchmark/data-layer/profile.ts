// `profile`: per-file wall times of single-file applies, for every rule alone, all rules together and the shared
// baselines, and what they add up to: per-rule totals, the slowest files and the files over the per-file budget.
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { cpus, tmpdir } from "node:os";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { z } from "zod";
import { baselineKind, compareStrings, compileBaselineRule, RuleWorkspace, type BaselineKind, type Engine, type PortedRule, type SourceFile } from "./engine.js";
import type { Repository } from "./corpus.js";

/** The per-file budget the report marks: a file over it would lose findings under a wall-clock deadline. */
const BUDGET_MS = 500;

const timingSchema = z.object({ ms: z.number(), findings: z.number().int(), failures: z.array(z.string()) });
type FileTiming = z.infer<typeof timingSchema>;
const passResultSchema = z.object({
  id: z.string(),
  files: z.record(z.string(), timingSchema),
  errors: z.array(z.object({ path: z.string(), message: z.string() })),
  wallSeconds: z.number(),
});
type PassResult = z.infer<typeof passResultSchema>;

/** A pass: `baseline-parse`, `baseline-text`, `all`, or `rule:<linter>`. */
export function passIds(rules: readonly PortedRule[]): string[] {
  const kinds = new Set(rules.map(rule => baselineKind(rule.document)));
  return [...[...kinds].sort(compareStrings).map(kind => `baseline-${kind}`), "all", ...rules.map(rule => `rule:${rule.linter}`)];
}

function passRules(engine: Engine, id: string, rules: readonly PortedRule[]): PortedRule[] {
  if (id === "all") return [...rules];
  if (id === "baseline-parse" || id === "baseline-text") return [compileBaselineRule(engine, id.slice("baseline-".length) as BaselineKind)];
  const rule = rules.find(candidate => `rule:${candidate.linter}` === id);
  if (rule === undefined) throw new Error(`Unknown pass ${id}`);
  return [rule];
}

/**
 * Times every accepted file on its own: apply it alone (timed: the native `applyAsync` call), then remove it
 * (untimed), so each timed apply evaluates exactly one file. One untimed warm-up apply per workspace pays one-time
 * costs (grammars, query plans) first. Keeps the fastest of `repeat` rounds.
 */
export async function runPass(engine: Engine, id: string, rules: readonly PortedRule[], files: readonly SourceFile[], repeat: number): Promise<PassResult> {
  const started = performance.now();
  const loaded = passRules(engine, id, rules);
  const timings: Record<string, FileTiming> = {};
  const errors: PassResult["errors"] = [];
  for (let round = 0; round < repeat; round += 1) {
    let workspace = new RuleWorkspace(engine, loaded);
    const accepted = files.filter(file => workspace.accepts(file.path));
    if (accepted[0] !== undefined) {
      await workspace.apply([accepted[0]]);
      await workspace.apply([], [accepted[0].path]);
    }
    for (const file of accepted) {
      try {
        const { wallMs, result } = await workspace.apply([file]);
        await workspace.apply([], [file.path]);
        const previous = timings[file.path];
        const failures = result.failures.map(failure => `${failure.ruleId ?? ""}/${failure.clauseId} ${failure.code}: ${failure.message}`);
        if (previous === undefined || wallMs < previous.ms) timings[file.path] = { ms: wallMs, findings: result.violations.length, failures };
      } catch (error) {
        errors.push({ path: file.path, message: error instanceof Error ? error.message : String(error) });
        // The workspace's revision may no longer match ours: continue on a fresh one.
        workspace.close();
        workspace = new RuleWorkspace(engine, loaded);
      }
    }
    workspace.close();
  }
  return { id, files: timings, errors, wallSeconds: (performance.now() - started) / 1000 };
}

/** Writes one pass's result where the parent reads it (`profile-pass`). */
export function writePassResult(path: string, result: PassResult): void {
  writeFileSync(path, JSON.stringify(result));
}

/**
 * Runs the passes, `jobs` at a time: with 1 here, one after another; otherwise each in a child process
 * (`data-layer.ts profile-pass`), since the addon runs one apply per process at a time.
 */
export async function runPasses(options: { engine: Engine; rules: PortedRule[]; files: SourceFile[]; ids: string[]; repeat: number; jobs: number; script: string; childArguments: string[] }): Promise<PassResult[]> {
  const log = (result: PassResult) => process.stderr.write(`pass ${result.id}: ${Object.keys(result.files).length} files in ${result.wallSeconds.toFixed(1)} s\n`);
  if (options.jobs <= 1) {
    const results: PassResult[] = [];
    for (const id of options.ids) {
      const result = await runPass(options.engine, id, options.rules, options.files, options.repeat);
      log(result);
      results.push(result);
    }
    return results;
  }
  const directory = mkdtempSync(resolve(tmpdir(), "rerail-data-layer-profile-"));
  try {
    const results = new Array<PassResult>(options.ids.length);
    let next = 0;
    const worker = async () => {
      while (next < options.ids.length) {
        const index = next++;
        const id = options.ids[index]!;
        const output = resolve(directory, `${index}.json`);
        await new Promise<void>((resolveRun, reject) => {
          const child = spawn(process.execPath, [...process.execArgv, options.script, "profile-pass", ...options.childArguments, "--pass", id, "--pass-result", output, "--repeat", String(options.repeat)], { stdio: ["ignore", "inherit", "inherit"] });
          child.on("error", reject);
          child.on("close", code => (code === 0 ? resolveRun() : reject(new Error(`pass ${id} exited ${code}`))));
        });
        results[index] = passResultSchema.parse(JSON.parse(readFileSync(output, "utf8")));
        log(results[index]);
      }
    };
    await Promise.all(Array.from({ length: Math.min(options.jobs, options.ids.length) }, worker));
    return results;
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

const ms = (value: number) => `${value.toFixed(1)} ms`;

/** `profile.md` and `profile.json` under `output`. */
export function writeProfile(options: { engine: Engine; repository: Repository; rules: PortedRule[]; results: PassResult[]; output: string; repeat: number; jobs: number; top: number }): void {
  const { engine, repository, rules, results, output, top } = options;
  const byId = new Map(results.map(result => [result.id, result]));
  const all = byId.get("all");
  if (all === undefined) throw new Error("The profile needs the `all` pass");
  const baseline = (kind: BaselineKind, path: string) => byId.get(`baseline-${kind}`)?.files[path]?.ms ?? 0;
  /** A rule's own cost on a file: its time alone minus its baseline there (negative noise counts as 0). */
  const own = rules.map(rule => {
    const kind = baselineKind(rule.document);
    const files = byId.get(`rule:${rule.linter}`)?.files ?? {};
    const costs: Record<string, number> = {};
    for (const [path, timing] of Object.entries(files)) costs[path] = Math.max(0, timing.ms - baseline(kind, path));
    const total = Object.values(costs).reduce((sum, value) => sum + value, 0);
    const scope = Object.keys(costs).length;
    return { linter: rule.linter, kind, clauses: rule.clauses, files: scope, totalMs: total, meanMs: scope === 0 ? 0 : total / scope, costs };
  }).sort((left, right) => right.totalMs - left.totalMs);
  const allTotal = Object.values(all.files).reduce((sum, timing) => sum + timing.ms, 0);
  const ownTotal = own.reduce((sum, rule) => sum + rule.totalMs, 0);
  const slowest = Object.entries(all.files).sort(([, left], [, right]) => right.ms - left.ms);
  const over = slowest.filter(([, timing]) => timing.ms > BUDGET_MS);
  const lines: string[] = [];
  lines.push(`# Data-layer per-file profile: ${rules.length} rules on nera-data-layer`, "");
  lines.push(`- ${over.length} of ${slowest.length} files need more than ${BUDGET_MS} ms with every rule loaded; the slowest needs ${ms(slowest[0]?.[1].ms ?? 0)} (${slowest[0]?.[0] ?? "—"}).`);
  lines.push(`- Σ per-file time with every rule loaded: ${(allTotal / 1000).toFixed(2)} s; Σ rules' own costs: ${(ownTotal / 1000).toFixed(2)} s.`, "");
  lines.push(`## Files over ${BUDGET_MS} ms (every rule loaded)`, "");
  if (over.length === 0) lines.push("None.");
  for (const [path, timing] of over) lines.push(`- \`${path}\`: ${ms(timing.ms)}`);
  // Only the baselines some pass measured: a catalog without regex-only rules has no text baseline.
  const kinds = (["parse", "text"] as const).filter(kind => byId.has(`baseline-${kind}`));
  lines.push("", `## Slowest ${top} files (every rule loaded)`, "", `| File | All rules | ${kinds.map(kind => `${kind} baseline | `).join("")}Top rules (own cost) |`, `| --- | --- | ${kinds.map(() => "--- | ").join("")}--- |`);
  for (const [path, timing] of slowest.slice(0, top)) {
    const ranked = own.filter(rule => rule.costs[path] !== undefined).sort((left, right) => right.costs[path]! - left.costs[path]!).slice(0, 3);
    lines.push(`| \`${path}\` | ${ms(timing.ms)} | ${kinds.map(kind => `${ms(baseline(kind, path))} | `).join("")}${ranked.map(rule => `${rule.linter} ${ms(rule.costs[path]!)}`).join(", ")} |`);
  }
  lines.push("", "## Rules ranked by total own cost", "", "| Rule | Baseline | Clauses | Files | Total | Mean / file | Share |", "| --- | --- | --- | --- | --- | --- | --- |");
  for (const rule of own) lines.push(`| ${rule.linter} | ${rule.kind} | ${rule.clauses} | ${rule.files} | ${ms(rule.totalMs)} | ${rule.meanMs.toFixed(2)} ms | ${ownTotal === 0 ? "—" : `${((rule.totalMs * 100) / ownTotal).toFixed(1)}%`} |`);
  lines.push("", "## Failures and errors (every rule loaded)", "");
  const failures = Object.entries(all.files).flatMap(([path, timing]) => timing.failures.map(failure => `\`${path}\` ${failure}`));
  if (failures.length === 0 && all.errors.length === 0) lines.push("None.");
  for (const failure of failures) lines.push(`- failure ${failure}`);
  for (const error of all.errors) lines.push(`- error \`${error.path}\`: ${error.message}`);
  lines.push("", "## Method", "");
  lines.push(`- Corpus: \`${repository.root}\` at \`${repository.git.commit}\`${repository.git.changes.length > 0 ? " (with working-tree changes)" : ""}; ${repository.files.length} Python files.`);
  lines.push(`- Addon: \`${engine.addon.path}\` (${engine.addon.sha256}, kernel ${engine.addon.kernelVersion}, work ${engine.addon.work}); ${engine.workerThreads} engine worker threads.`);
  lines.push(`- Per-file time: the wall time of one native \`applyAsync\` that evaluates that file alone (then an untimed removal), fastest of ${options.repeat} run(s); ${options.jobs} pass(es) at a time.`);
  lines.push(`- A rule's own cost: its time alone minus its baseline (parse for a rule with any AST query, text for a regex-only rule).`);
  lines.push(`- Machine: ${cpus()[0]?.model ?? "unknown"}, ${cpus().length} logical CPUs, ${process.platform} ${process.arch}, Node ${process.version}.`);
  mkdirSync(output, { recursive: true });
  writeFileSync(resolve(output, "profile.md"), `${lines.join("\n")}\n`);
  writeFileSync(resolve(output, "profile.json"), `${JSON.stringify({
    format: "rerail.data-layer-profile/v1",
    addon: engine.addon,
    workerThreads: engine.workerThreads,
    corpus: { root: repository.root, commit: repository.git.commit, files: repository.files.length },
    repeat: options.repeat,
    jobs: options.jobs,
    rules: own.map(({ costs: _costs, ...rule }) => rule),
    passes: results,
  })}\n`);
}
