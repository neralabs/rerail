/** Measure all selected ports together against the upstream shared fact engine's repository snapshot. */
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { parseArgs } from "node:util";
import { z } from "zod";
import { codeRuleSchema, encodeCodeRuleWorkspaceRequest, loadNativeBinding, resolveNativeBindingPath, RESULT_PAGE_BYTES } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";
import { escapeHatchSignatureFailure } from "./signed-allowlist.js";

const recordingSchema = z.object({
  commit: z.string(), workers: z.number(), oracle_ms: z.number().nullable(), source_mismatches: z.array(z.string()),
  files: z.array(z.object({ path: z.string(), source: z.string() })),
  events: z.array(z.object({ check: z.string(), findings: z.array(z.object({ path: z.string(), line: z.number(), code: z.string() })) })),
});
const pageSchema = z.object({ done: z.boolean(), cursor: z.number(),
  violations: z.array(z.object({ ruleId: z.string(), path: z.string(), code: z.string(), span: z.object({ startLine: z.number() }) })),
  failures: z.array(z.unknown()),
});
const { values } = parseArgs({ options: { recording: { type: "string" }, output: { type: "string" }, allocator: { type: "string", default: "unspecified" },
  policies: { type: "string", default: "packs/data-layer/candidates/rules" }, progress: { type: "boolean", default: false }, "cold-only": { type: "boolean", default: false }, "memory-census": { type: "boolean", default: false }, "batch-files": { type: "string", default: "32" } } });
if (values.recording === undefined || values.output === undefined) throw new Error("Usage: suite.ts --recording <json> --output <json>");
const batchFiles = Number(values["batch-files"]);
if (!Number.isInteger(batchFiles) || batchFiles < 1 || batchFiles > 2048) throw new Error("--batch-files must be an integer between 1 and 2048");
const recording = recordingSchema.parse(JSON.parse(readFileSync(values.recording, "utf8")));
const workloadSha256 = createHash("sha256").update(JSON.stringify({ commit: recording.commit, files: recording.files, events: recording.events })).digest("hex");
if (recording.source_mismatches.length !== 0) throw new Error("Pinned source verification failed");
const native = loadNativeBinding();
const nativeSha256 = createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex");
const workspace = new native.CodeRuleDslWorkspace();
const policyDigests: Record<string, string> = {};
const policyCompilationMs: Record<string, number> = {};
const phaseCpuMs: Record<string, { userMs: number; systemMs: number }> = {};
const memoryCheckpoints: unknown[] = [];
const applies: { revision: number; phase: string; files: number; sourceBytes: number; wallMs: number; cpuMs: number; rssBytes: number; heapUsed: number; external: number; arrayBuffers: number }[] = [];
const memoryCheckpoint = (phase: string): void => {
  if (!values["memory-census"]) return;
  const cpu = process.cpuUsage(), started = performance.now();
  const workspaceMemory: unknown = JSON.parse(workspace.memoryProfile());
  const elapsed = process.cpuUsage(cpu);
  memoryCheckpoints.push({ phase, revision, process: process.memoryUsage(), workspace: workspaceMemory,
    censusWallMs: performance.now() - started, censusCpuMs: (elapsed.user + elapsed.system) / 1000 });
};
const finishCpu = (phase: string, started: NodeJS.CpuUsage): void => {
  const elapsed = process.cpuUsage(started);
  phaseCpuMs[phase] = { userMs: elapsed.user / 1000, systemMs: elapsed.system / 1000 };
};
const markPhase = (phase: string): void => {
  if (process.env.RERAIL_PROFILE_PHASES !== undefined) console.error(`RERAIL_PROFILE_PHASE ${JSON.stringify({ phase, startUnixUs: Date.now() * 1000 })}`);
};
const files = recording.files.map(file => ({ ...file, language: file.path.endsWith(".rs") ? "rust" : "python" }));
let revision = 0;
let loadedFiles = 0;
const send = async (batchFiles: typeof files, deferGlobal: boolean): Promise<void> => {
  const cpu = process.cpuUsage(), started = performance.now();
  await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: ++revision, files: batchFiles, removed: [], quiet: true, deferGlobal }));
  loadedFiles += batchFiles.length;
  const elapsed = process.cpuUsage(cpu), memory = process.memoryUsage();
  applies.push({ revision, phase: deferGlobal ? "file-inputs" : "global-results", files: batchFiles.length,
    sourceBytes: batchFiles.reduce((sum, file) => sum + Buffer.byteLength(file.source), 0),
    wallMs: performance.now() - started, cpuMs: (elapsed.user + elapsed.system) / 1000,
    rssBytes: memory.rss, heapUsed: memory.heapUsed, external: memory.external, arrayBuffers: memory.arrayBuffers });
  if (values.progress) console.error(JSON.stringify({ phase: deferGlobal ? "file-inputs" : "global-results", loadedFiles, revision, rssBytes: process.memoryUsage().rss }));
};
const collect = async (): Promise<{ violations: z.infer<typeof pageSchema>["violations"]; failures: unknown[] }> => {
  const violations: z.infer<typeof pageSchema>["violations"] = [], failures: unknown[] = [];
  for (let cursor = 0; ;) {
    const page = pageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
    violations.push(...page.violations); failures.push(...page.failures);
    if (page.done) return { violations, failures };
    if (page.cursor <= cursor) throw new Error("Native result cursor did not advance");
    cursor = page.cursor;
  }
};
const findingKeys = (rows: z.infer<typeof pageSchema>["violations"]): string[] => rows.map(f => JSON.stringify([f.ruleId, f.path, f.span.startLine + 1, f.code])).sort();
try {
  markPhase("compile");
  const compileCpu = process.cpuUsage();
  const compileStarted = performance.now();
  for (const event of recording.events) {
    const id = `data-layer-${event.check.replaceAll("_", "-")}`;
    const policyStarted = performance.now();
    const source = readFileSync(`${values.policies}/${id}.json`, "utf8");
    if (event.check === "sql_grammar_escape_hatches" && escapeHatchSignatureFailure(codeRuleSchema.parse(JSON.parse(source))) !== undefined) {
      throw new Error("Escape-hatch allowlist signature verification failed");
    }
    const compiled = native.CodeRuleDslRuntime.compile(source, "json", capabilitiesJson());
    policyCompilationMs[id] = performance.now() - policyStarted;
    policyDigests[id] = compiled.digest();
    workspace.addCodeRule(compiled);
  }
  const compilationMs = performance.now() - compileStarted;
  finishCpu("compile", compileCpu);
  markPhase("cold");
  const coldCpu = process.cpuUsage();
  const started = performance.now();
  let batch: typeof files = [], bytes = 0;
  for (const file of files) {
    batch.push(file);
    bytes += Buffer.byteLength(file.source);
    if (batch.length >= batchFiles || bytes >= 1024 * 1024) { await send(batch, true); batch = []; bytes = 0; }
  }
  if (batch.length > 0) await send(batch, true);
  memoryCheckpoint("inputs-retained");
  await send([], false);
  memoryCheckpoint("global-complete");
  const { violations, failures } = await collect();
  memoryCheckpoint("results-paged");
  const coldMs = performance.now() - started;
  finishCpu("cold", coldCpu);
  const file = files.find(file => file.path === "impl/common/l2200_sql_dialect/sql_connection.py") ?? files[0]!;
  const incrementalResults: { phase: string; result: Awaited<ReturnType<typeof collect>> }[] = [];
  let unchangedFileMs: number | null = null, editedFileMs: number | null = null;
  if (!values["cold-only"]) {
    markPhase("unchanged");
    const unchangedCpu = process.cpuUsage(), resendStarted = performance.now();
    await send([file], false);
    const unchangedResult = await collect();
    unchangedFileMs = performance.now() - resendStarted;
    finishCpu("unchanged", unchangedCpu);
    incrementalResults.push({ phase: "unchanged", result: unchangedResult });
    markPhase("edit");
    const editCpu = process.cpuUsage(), editStarted = performance.now();
    await send([{ ...file, source: file.source + "\n" }], false);
    const editedResult = await collect();
    editedFileMs = performance.now() - editStarted;
    finishCpu("edit", editCpu);
    incrementalResults.push({ phase: "trailing-newline-edit", result: editedResult });
  }
  markPhase("report");
  if (values["memory-census"]) { workspace.close(); memoryCheckpoint("workspace-closed"); }
  const expected = recording.events.flatMap(event => event.findings.map(f => JSON.stringify([`data-layer-${event.check.replaceAll("_", "-")}`, f.path, f.line, f.code]))).sort();
  const actual = findingKeys(violations);
  const difference = (a: string[], b: string[]) => { const rest = [...b]; return a.filter(key => { const at = rest.indexOf(key); if (at < 0) return true; rest.splice(at, 1); return false; }); };
  const incrementalDifferences = incrementalResults.flatMap(({ phase, result }) => {
    const keys = findingKeys(result.violations);
    const falsePositives = difference(keys, expected), falseNegatives = difference(expected, keys);
    return falsePositives.length + falseNegatives.length + result.failures.length === 0 ? []
      : [{ phase, falsePositives, falseNegatives, failures: result.failures }];
  });
  const report = { commit: recording.commit, files: files.length, bytes: files.reduce((sum, file) => sum + Buffer.byteLength(file.source), 0),
    oracleMs: recording.oracle_ms, oracleWorkers: recording.workers, policyDigests, policyCompilationMs, compilationMs, coldMs, unchangedFileMs, editedFileMs, editedPath: file.path,
    incrementalDifferences, phaseCpuMs, measuredPhases: Object.keys(phaseCpuMs), workloadSha256,
    expected: expected.length, actual: actual.length, falsePositives: difference(actual, expected), falseNegatives: difference(expected, actual), failures,
    peakRssBytes: process.resourceUsage().maxRSS * 1024,
    allocator: values.allocator, policies: values.policies, nativeSha256, nativeWorkers: 4,
    batchFiles, applies, memoryCheckpoints, measurementNote: "Phase CPU includes explicit census overhead when --memory-census is supplied. Reference timings must omit this option." };
  writeFileSync(values.output, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ ...report, falsePositives: report.falsePositives.length, falseNegatives: report.falseNegatives.length, failures: failures.length, incrementalDifferences: incrementalDifferences.length }, null, 2));
  if (report.falsePositives.length + report.falseNegatives.length + failures.length + incrementalDifferences.length !== 0) process.exitCode = 1;
} finally { workspace.close(); }
