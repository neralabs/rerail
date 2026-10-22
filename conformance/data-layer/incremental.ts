/** Replay original positive/clean analyzer inputs as edits in one resident workspace, its AST clauses matched by the selected engine. */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { z } from "zod";
import { codeRuleSchema, encodeCodeRuleWorkspaceRequest, loadNativeBinding, resolveNativeBindingPath, RESULT_PAGE_BYTES } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";
import { factsBinarySha256, runFactsCalls, type FactsCall } from "./facts-engine.js";

const eventSchema = z.object({
  test: z.string(), check: z.string(), entry: z.string(), upstream_passed: z.boolean(), retained_reason: z.string().optional(),
  production_scope: z.boolean().optional(), scope: z.object({ include: z.array(z.string()), exclude: z.array(z.string()) }).optional(),
  oracle_config: z.object({ options: z.record(z.string(), z.unknown()).optional() }).optional(),
  files: z.array(z.object({ path: z.string(), source: z.string() })).optional(),
  findings: z.array(z.object({ path: z.string(), line: z.number(), code: z.string() })).optional(),
});
const recordingSchema = z.object({ commit: z.string(), pytest_exit_code: z.literal(0), source_mismatches: z.array(z.string()).max(0), events: z.array(eventSchema) });
const pageSchema = z.object({ done: z.boolean(), cursor: z.number(), failures: z.array(z.unknown()), violations: z.array(z.object({ path: z.string(), code: z.string(), span: z.object({ startLine: z.number() }) })) });
const authoredSchema = z.object({ clauses: z.array(z.object({ define: z.object({ options: z.record(z.string(), z.unknown()).optional(), config: z.record(z.string(), z.unknown()).optional() }).passthrough().optional() }).passthrough()) }).passthrough();
const { values } = parseArgs({ options: { recording: { type: "string", multiple: true }, output: { type: "string" }, policies: { type: "string", default: "packs/data-layer/candidates/rules" }, engine: { type: "string", default: "native" } } });
if (values.recording === undefined || values.output === undefined) throw new Error("Usage: incremental.ts --recording <json> --output <json> [--engine native|sql|facts]");
const engine = values.engine;
if (engine !== "native" && engine !== "sql" && engine !== "facts") throw new Error("--engine must be native, sql or facts");
const recordings = values.recording.map(path => recordingSchema.parse(JSON.parse(readFileSync(path, "utf8"))));
const commit = recordings[0]!.commit;
if (recordings.some(recording => recording.commit !== commit)) throw new Error("Recordings must share a source pin");
const plan = z.object({ batches: z.array(z.object({ id: z.number(), checks: z.array(z.string()) })) }).parse(JSON.parse(readFileSync("fixtures/host/data-layer-port/batches.json", "utf8")));
const checks = plan.batches.find(batch => batch.id === 6)!.checks;
const events = recordings.flatMap(recording => recording.events).filter(event => event.upstream_passed && event.retained_reason === undefined && !event.production_scope && event.files !== undefined && event.findings !== undefined && event.scope !== undefined);
// The facts engine has no resident workspace: each transition is a from-scratch evaluation of that input (`facts-engine.ts`).
const native = engine === "facts" ? undefined : loadNativeBinding();
const nativeSha256 = engine === "facts" ? null : createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex");
const factsCalls: (FactsCall & { check: string; cleanTest: string; positiveTest: string; phase: string; expected: string[] })[] = [];
const rows = [];
let failed = 0;
const started = performance.now();
const subtract = (left: string[], right: string[]) => {
  const remaining = [...right];
  return left.filter(key => { const at = remaining.indexOf(key); if (at < 0) return true; remaining.splice(at, 1); return false; });
};
for (const check of checks) {
  const candidates = events.filter(event => event.check === check);
  const pair = candidates.filter(event => event.findings!.length > 0).map(positive => ({ positive, clean: candidates.find(clean => clean.findings!.length === 0 && clean.entry === positive.entry && JSON.stringify(clean.oracle_config) === JSON.stringify(positive.oracle_config) && JSON.stringify(clean.scope) === JSON.stringify(positive.scope)) })).find(pair => pair.clean !== undefined);
  if (pair?.clean === undefined) throw new Error(`${check} needs original positive and clean inputs with the same analyzer configuration`);
  const authored = authoredSchema.parse(JSON.parse(readFileSync(`${values.policies}/data-layer-${check.replaceAll("_", "-")}.json`, "utf8")));
  for (const clause of authored.clauses) if (clause.define?.options !== undefined) {
    clause.define.config = { ...clause.define.config, oracleAnalyzer: true, entryPoint: pair.positive.entry, ...pair.positive.oracle_config?.options };
  }
  const policy = codeRuleSchema.parse(authored);
  if (pair.positive.scope === undefined) throw new Error(`${check} has no recorded source scope`);
  policy.scope = pair.positive.scope;
  if (engine === "facts") {
    for (const [index, event] of [pair.clean, pair.positive, pair.clean, pair.positive].entries()) {
      factsCalls.push({ id: factsCalls.length, policy, files: event.files!, check, cleanTest: pair.clean.test, positiveTest: pair.positive.test,
        phase: index % 2 === 0 ? "clean" : "positive", expected: event.findings!.map(finding => JSON.stringify([finding.path, finding.line, finding.code])) });
    }
    continue;
  }
  const rule = native!.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilitiesJson());
  const workspace = new native!.CodeRuleDslWorkspace({ engine });
  const transitions = [];
  let paths = new Set<string>();
  try {
    workspace.addCodeRule(rule);
    for (const [index, event] of [pair.clean, pair.positive, pair.clean, pair.positive].entries()) {
      const incoming = new Set(event.files!.map(file => file.path));
      await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: index + 1,
        files: event.files!.map(file => ({ ...file, language: "python" })), removed: [...paths].filter(path => !incoming.has(path)), quiet: true }));
      paths = incoming;
      const actual: string[] = [], failures: unknown[] = [];
      for (let cursor = 0; ;) {
        const page = pageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
        actual.push(...page.violations.map(finding => JSON.stringify([finding.path, finding.span.startLine + 1, finding.code])));
        failures.push(...page.failures);
        if (page.done) break;
        if (page.cursor <= cursor) throw new Error("Native result cursor did not advance");
        cursor = page.cursor;
      }
      const expected = event.findings!.map(finding => JSON.stringify([finding.path, finding.line, finding.code]));
      const falsePositives = subtract(actual, expected), falseNegatives = subtract(expected, actual);
      failed += falsePositives.length + falseNegatives.length + failures.length;
      transitions.push({ phase: index % 2 === 0 ? "clean" : "positive", expected: expected.length, actual: actual.length, falsePositives, falseNegatives, failures });
    }
  } finally { workspace.close(); }
  rows.push({ check, cleanTest: pair.clean.test, positiveTest: pair.positive.test, transitions });
}
if (engine === "facts") {
  const results = runFactsCalls(factsCalls.map(({ id, policy, files }) => ({ id, policy, files })));
  for (const call of factsCalls) {
    const { result } = results.get(call.id)!;
    const actual = result.violations.map(finding => JSON.stringify([finding.path, finding.span.startLine + 1, finding.code]));
    const falsePositives = subtract(actual, call.expected), falseNegatives = subtract(call.expected, actual);
    failed += falsePositives.length + falseNegatives.length + result.failures.length;
    const transition = { phase: call.phase, expected: call.expected.length, actual: actual.length, falsePositives, falseNegatives, failures: result.failures };
    const row = rows.find(row => row.check === call.check);
    if (row === undefined) rows.push({ check: call.check, cleanTest: call.cleanTest, positiveTest: call.positiveTest, transitions: [transition] });
    else row.transitions.push(transition);
  }
}
const report = { format: "rerail.data-layer-incremental-parity/v1", commit, nativeSha256, factsSha256: engine === "facts" ? factsBinarySha256() : null, engine, checks: rows.length, transitions: rows.length * 4, differences: failed, elapsedMs: performance.now() - started, rows };
writeFileSync(values.output, JSON.stringify(report, null, 2) + "\n");
console.log(JSON.stringify({ ...report, rows: undefined }, null, 2));
if (failed !== 0) process.exitCode = 1;
