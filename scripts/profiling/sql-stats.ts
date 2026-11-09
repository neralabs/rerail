/** Read SQL cardinalities through a diagnostic copy of an authored policy. */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { pathToFileURL } from "node:url";
import { z } from "zod";
import { encodeCodeRuleWorkspaceRequest, loadNativeBinding, resolveNativeBindingPath, RESULT_PAGE_BYTES } from "../../src/engine/index.js";
import { capabilitiesJson } from "../../conformance/engine.js";

const authoredSchema = z.object({ id: z.string(), clauses: z.array(z.object({
  define: z.unknown().optional(),
  relational: z.object({ inputs: z.record(z.string(), z.unknown()),
    stages: z.array(z.object({ name: z.string() }).passthrough()).optional(), query: z.unknown(),
    key: z.unknown().optional(), code: z.unknown().optional(),
  }).passthrough().optional(),
}).passthrough()) }).passthrough();
const metricSchema = z.object({ relation: z.string(), metric: z.string(), query: z.string() }).strict();
export const metricsSchema = z.record(z.string(), z.array(metricSchema));
type Metrics = z.infer<typeof metricsSchema>;
const identifier = (value: string): string => `"${value.replaceAll('"', '""')}"`;
const literal = (value: string): string => `'${value.replaceAll("'", "''")}'`;

/** Change only final diagnostic SELECTs; SQLite executes all original producers. */
export function statisticsPolicy(source: unknown, metrics: Metrics = {}, tables?: Set<string>): unknown {
  const document = authoredSchema.parse(source);
  document.clauses = document.clauses.filter(clause => {
    const plan = clause.relational;
    if (plan === undefined) return clause.define !== undefined;
    const relations = [...Object.keys(plan.inputs), ...plan.stages?.map(stage => stage.name) ?? []];
    const custom = metrics[String(clause.id)] ?? [];
    for (const item of custom) if (!relations.includes(item.relation)) throw new Error(`Unknown metric relation: ${item.relation}`);
    const selected = relations.filter(name => tables === undefined || tables.has(name));
    const items = [...selected.map(relation => ({ relation, metric: "rows", query: `SELECT count(*) FROM ${identifier(relation)}` })),
      ...custom.filter(item => tables === undefined || tables.has(item.relation))];
    if (items.length === 0) return false;
    const anchors = Object.keys(plan.inputs).map(name => `SELECT __path,__start,__end FROM ${identifier(name)}`).join(" UNION ALL ");
    const counts = items.map(item => `SELECT ${literal(item.relation)} AS relation,${literal(item.metric)} AS metric,(${item.query}) AS value`).join(" UNION ALL ");
    const query = `WITH __profile_anchor AS (${anchors} LIMIT 1),__profile_counts AS (${counts})
SELECT a.__path AS path,a.__start AS start_byte,a.__end AS end_byte,c.relation,c.metric,c.value
FROM __profile_anchor a CROSS JOIN __profile_counts c`;
    if (Buffer.byteLength(query) > 16_384) throw new Error("Statistics SELECT exceeds the SQL limit; restrict --tables");
    plan.query = query;
    plan.key = ["relation", "metric"];
    delete plan.code;
    clause.violation = { code: "SQL_PROFILE", message: "SQL profiling statistic", anchor: "witness.span" };
    return true;
  });
  return document;
}

async function main(): Promise<void> {
  const { values } = parseArgs({ options: { policy: { type: "string" }, recording: { type: "string" }, output: { type: "string" },
    metrics: { type: "string" }, tables: { type: "string" } } });
  if (!values.policy || !values.recording || !values.output) throw new Error("Usage: sql-stats.ts --policy <json> --recording <json> --output <json> [--metrics <json>] [--tables <names>]");
  const source = readFileSync(values.policy, "utf8");
  const original = JSON.parse(source) as unknown;
  const metrics = values.metrics ? metricsSchema.parse(JSON.parse(readFileSync(values.metrics, "utf8"))) : {};
  const policy = statisticsPolicy(original, metrics, values.tables ? new Set(values.tables.split(",")) : undefined);
  const recording = z.object({ commit: z.string(), source_mismatches: z.array(z.string()).max(0),
    files: z.array(z.object({ path: z.string(), source: z.string() })) }).parse(JSON.parse(readFileSync(values.recording, "utf8")));
  const pageSchema = z.object({ done: z.boolean(), cursor: z.number(), failures: z.array(z.unknown()),
    violations: z.array(z.object({ path: z.string(), clauseId: z.string(), variables: z.object({ row: z.object({
      relation: z.string(), metric: z.string(), value: z.union([z.number(), z.string(), z.null()]),
    }) }) })) });
  const native = loadNativeBinding(), workspace = new native.CodeRuleDslWorkspace();
  const started = performance.now();
  const rows: { clause: string; path: string; relation: string; metric: string; value: number | string | null }[] = [];
  const failures: unknown[] = [];
  try {
    workspace.addCodeRule(native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilitiesJson()));
    const files = recording.files.map(file => ({ ...file, language: file.path.endsWith(".rs") ? "rust" : "python" }));
    let revision = 0, bytes = 0, batch: typeof files = [];
    for (const file of files) {
      batch.push(file); bytes += Buffer.byteLength(file.source);
      if (batch.length >= 32 || bytes >= 1024 * 1024) {
        await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: ++revision, files: batch, removed: [], quiet: true, deferGlobal: true }));
        batch = []; bytes = 0;
      }
    }
    await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: ++revision, files: batch, removed: [], quiet: true, deferGlobal: true }));
    await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: ++revision, files: [], removed: [], quiet: true, deferGlobal: false }));
    for (let cursor = 0; ;) {
      const page = pageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
      failures.push(...page.failures);
      rows.push(...page.violations.map(row => ({ clause: row.clauseId, path: row.path, ...row.variables.row })));
      if (page.done) break;
      if (page.cursor <= cursor) throw new Error("Native result cursor did not advance");
      cursor = page.cursor;
    }
  } finally { workspace.close(); }
  const report = { format: "rerail.sql-statistics/v1", mode: "diagnostic-policy-copy", commit: recording.commit,
    nativeSha256: createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex"),
    originalPolicySha256: createHash("sha256").update(source).digest("hex"),
    filesSha256: createHash("sha256").update(JSON.stringify(recording.files)).digest("hex"),
    elapsedMs: performance.now() - started, rows, failures,
    note: "Diagnostic SELECTs replace findings and add SQL work. This is not a parity run, benchmark, or physical-memory measurement." };
  writeFileSync(values.output, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ ...report, rows: rows.length }));
  if (failures.length > 0) process.exitCode = 1;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { console.error(error); process.exitCode = 1; });
}
