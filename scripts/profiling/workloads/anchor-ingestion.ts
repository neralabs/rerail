/** Bounded ingestion controls. Findings parity and repository-wide evaluation are separate gates. */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { loadNativeBinding, resolveNativeBindingPath, encodeCodeRuleWorkspaceRequest, RESULT_PAGE_BYTES } from "../../../src/engine/index.js";
import { capabilitiesJson } from "../../../conformance/engine.js";

const { values } = parseArgs({ options: {
  recording: { type: "string" }, policies: { type: "string", default: "packs/data-layer/candidates/rules" },
  files: { type: "string", default: "128" }, rules: { type: "string", default: "50" }, output: { type: "string" },
  "memory-census": { type: "boolean", default: false },
} });
if (!values.recording || !values.output) throw new Error("Required: --recording FILE --output FILE [--files N] [--rules 50|60]");
const bytes = readFileSync(values.recording);
const recording = JSON.parse(bytes.toString()) as {
  files: { path: string; source: string }[]; events: { check: string }[]; source_mismatches: string[];
};
const count = Number(values.files), rules = Number(values.rules);
if (!Number.isSafeInteger(count) || count < 1 || count > recording.files.length
    || ![50, 60].includes(rules) || recording.events.length < rules || recording.source_mismatches.length) {
  throw new Error("Invalid selection or unverified source recording");
}
// Sample across the whole repository, including the known large-file regression.
const indices = new Set(Array.from({ length: count }, (_, i) => Math.floor(i * recording.files.length / count)));
const sentinel = recording.files.findIndex(f => f.path === "impl/common/l2100_sql_grammar/rust_bridge/generated_facades.py");
if (sentinel >= 0 && !indices.has(sentinel)) {
  indices.delete([...indices].at(-1)!);
  indices.add(sentinel);
}
const files = [...indices].sort((a, b) => a - b).map(i => ({ ...recording.files[i]!,
  language: recording.files[i]!.path.endsWith(".rs") ? "rust" : "python" }));
const hash = (value: string | Uint8Array): string => createHash("sha256").update(value).digest("hex");
const phase = (name: string): void => {
  if (process.env.RERAIL_PROFILE_PHASES !== undefined) console.error(`RERAIL_PROFILE_PHASE ${JSON.stringify({ phase: name, startUnixUs: Date.now() * 1000 })}`);
};
const native = loadNativeBinding(), workspace = new native.CodeRuleDslWorkspace();
const policies: Record<string, string> = {};
const memoryCheckpoints: unknown[] = [];
const memoryCheckpoint = (name: string): void => {
  if (!values["memory-census"]) return;
  const cpu = process.cpuUsage(), started = performance.now();
  const owned: unknown = JSON.parse(workspace.memoryProfile());
  const elapsed = process.cpuUsage(cpu);
  memoryCheckpoints.push({ name, process: process.memoryUsage(), owned, native: JSON.parse(native.nativeMemoryProfile()),
    wallMs: performance.now() - started, cpuMs: (elapsed.user + elapsed.system) / 1000 });
};
try {
  memoryCheckpoint("empty");
  phase("compile");
  const compileCpu = process.cpuUsage(), compileStarted = performance.now();
  for (const { check } of recording.events.slice(0, rules)) {
    const name = `data-layer-${check.replaceAll("_", "-")}`;
    const source = readFileSync(`${values.policies}/${name}.json`, "utf8");
    const rule = native.CodeRuleDslRuntime.compile(source, "json", capabilitiesJson());
    policies[name] = rule.digest(); workspace.addCodeRule(rule);
  }
  const compilationMs = performance.now() - compileStarted, compilationCpu = process.cpuUsage(compileCpu);
  memoryCheckpoint("compiled");
  phase("ingestion");
  const started = performance.now(), cpu = process.cpuUsage();
  let batch: typeof files = [], size = 0, revision = 0;
  const send = async (): Promise<void> => {
    await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: ++revision, files: batch, removed: [], quiet: true, deferGlobal: true }));
    batch = []; size = 0;
  };
  for (const file of files) {
    batch.push(file); size += Buffer.byteLength(file.source);
    if (batch.length >= 32 || size >= 1024 * 1024) await send();
  }
  if (batch.length) await send();
  const ingestionMs = performance.now() - started, ingestionCpu = process.cpuUsage(cpu);
  memoryCheckpoint("ingested");
  phase("report");
  const failures: unknown[] = [];
  for (let cursor = 0; ;) {
    const page = JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES));
    failures.push(...page.failures);
    if (page.done) break;
    if (page.cursor <= cursor) throw new Error("Native result cursor did not advance");
    cursor = page.cursor;
  }
  const report = {
    format: "rerail.anchor-ingestion-control/v1", acceptance: false,
    note: "Ingestion only. Repository analyses and findings parity are not evaluated. RSS is the driver reading before cleanup; use OS resource accounting through exit.",
    files: files.length, rules, sourceBytes: files.reduce((n, f) => n + Buffer.byteLength(f.source), 0),
    recordingSha256: hash(bytes), selectionSha256: hash(JSON.stringify(files)),
    nativeSha256: hash(readFileSync(resolveNativeBindingPath())), policyDigests: policies,
    compilationMs, compilationCpuMs: (compilationCpu.user + compilationCpu.system) / 1000,
    ingestionMs, ingestionCpuMs: (ingestionCpu.user + ingestionCpu.system) / 1000,
    driverPeakRssBytes: process.resourceUsage().maxRSS * 1024, localFailures: failures,
    memoryCheckpoints,
  };
  writeFileSync(values.output, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ ...report, policyDigests: undefined, localFailures: failures.length }));
  if (failures.length) process.exitCode = 1;
} finally { workspace.close(); }
