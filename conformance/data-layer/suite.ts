/** Measure all selected ports together against the upstream shared fact engine's repository snapshot. */
import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { parseArgs } from "node:util";
import { z } from "zod";
import { codeRuleSchema, encodeCodeRuleWorkspaceRequest, loadNativeBinding, resolveNativeBindingPath, RESULT_PAGE_BYTES } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";
import { escapeHatchSignatureFailure } from "./signed-allowlist.js";

const recordingSchema = z.object({
  commit: z.string(), workers: z.number(), oracle_ms: z.number(), source_mismatches: z.array(z.string()),
  files: z.array(z.object({ path: z.string(), source: z.string() })),
  events: z.array(z.object({ check: z.string(), findings: z.array(z.object({ path: z.string(), line: z.number(), code: z.string() })) })),
});
const pageSchema = z.object({ done: z.boolean(), cursor: z.number(),
  violations: z.array(z.object({ ruleId: z.string(), path: z.string(), code: z.string(), span: z.object({ startLine: z.number() }) })),
  failures: z.array(z.unknown()),
});
const { values } = parseArgs({ options: { recording: { type: "string" }, output: { type: "string" }, allocator: { type: "string", default: "unspecified" },
  policies: { type: "string", default: "packs/data-layer/candidates/rules" } } });
if (values.recording === undefined || values.output === undefined) throw new Error("Usage: suite.ts --recording <json> --output <json>");
const recording = recordingSchema.parse(JSON.parse(readFileSync(values.recording, "utf8")));
if (recording.source_mismatches.length !== 0) throw new Error("Pinned source verification failed");
const native = loadNativeBinding();
const workspace = new native.CodeRuleDslWorkspace();
const policyDigests: Record<string, string> = {};
const policyCompilationMs: Record<string, number> = {};
const files = recording.files.map(file => ({ ...file, language: file.path.endsWith(".rs") ? "rust" : "python" }));
let revision = 0;
const send = async (batchFiles: typeof files, deferGlobal: boolean): Promise<void> => {
  await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: ++revision, files: batchFiles, removed: [], quiet: true, deferGlobal }));
};
try {
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
  const started = performance.now();
  let batch: typeof files = [], bytes = 0;
  for (const file of files) {
    batch.push(file);
    bytes += Buffer.byteLength(file.source);
    if (batch.length >= 32 || bytes >= 1024 * 1024) { await send(batch, true); batch = []; bytes = 0; }
  }
  if (batch.length > 0) await send(batch, true);
  await send([], false);
  const violations: z.infer<typeof pageSchema>["violations"] = [], failures: unknown[] = [];
  for (let cursor = 0; ;) {
    const page = pageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
    violations.push(...page.violations); failures.push(...page.failures);
    if (page.done) break;
    if (page.cursor <= cursor) throw new Error("Native result cursor did not advance");
    cursor = page.cursor;
  }
  const coldMs = performance.now() - started;
  const file = files.find(file => file.path === "impl/common/l2200_sql_dialect/sql_connection.py") ?? files[0]!;
  const resendStarted = performance.now();
  await send([file], false);
  const unchangedFileMs = performance.now() - resendStarted;
  const editStarted = performance.now();
  await send([{ ...file, source: file.source + "\n" }], false);
  const editedFileMs = performance.now() - editStarted;
  const expected = recording.events.flatMap(event => event.findings.map(f => JSON.stringify([`data-layer-${event.check.replaceAll("_", "-")}`, f.path, f.line, f.code]))).sort();
  const actual = violations.map(f => JSON.stringify([f.ruleId, f.path, f.span.startLine + 1, f.code])).sort();
  const difference = (a: string[], b: string[]) => { const rest = [...b]; return a.filter(key => { const at = rest.indexOf(key); if (at < 0) return true; rest.splice(at, 1); return false; }); };
  const report = { commit: recording.commit, files: files.length, bytes: files.reduce((sum, file) => sum + Buffer.byteLength(file.source), 0),
    oracleMs: recording.oracle_ms, oracleWorkers: recording.workers, policyDigests, policyCompilationMs, compilationMs, coldMs, unchangedFileMs, editedFileMs, editedPath: file.path,
    expected: expected.length, actual: actual.length, falsePositives: difference(actual, expected), falseNegatives: difference(expected, actual), failures,
    allocator: values.allocator, policies: values.policies, nativeSha256: createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex"), nativeWorkers: 4 };
  writeFileSync(values.output, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ ...report, falsePositives: report.falsePositives.length, falseNegatives: report.falseNegatives.length, failures: failures.length }, null, 2));
  if (report.falsePositives.length + report.falseNegatives.length + failures.length !== 0) process.exitCode = 1;
} finally { workspace.close(); }
