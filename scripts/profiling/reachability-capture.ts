/** Capture actual native SQL inputs for isolated SQLite reachability experiments. */
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";
import { z } from "zod";
import { capabilitiesJson } from "../../conformance/engine.js";
import { codeRuleSchema, loadNativeBinding, resolveNativeBindingPath } from "../../src/engine/index.js";

const inputSchema = z.object({ evidence: z.object({ project: z.record(z.string(), z.unknown()) }).passthrough() }).passthrough();
const sourceSchema = z.object({ clauses: z.array(z.object({ id: z.string(), define: z.object({
  config: z.record(z.string(), z.unknown()).optional(), options: z.record(z.string(), z.object({ default: z.unknown() }).passthrough()).optional(),
}).passthrough().optional(), relational: z.object({ inputs: z.record(z.string(), inputSchema),
  stages: z.array(z.unknown()).optional(), query: z.unknown(), key: z.unknown().optional(), code: z.unknown().optional(),
}).passthrough().optional() }).passthrough()) }).passthrough();
const filesSchema = z.array(z.object({ path: z.string(), source: z.string() }));
const resultSchema = z.object({ failures: z.array(z.unknown()), violations: z.array(z.object({
  path: z.string(), code: z.string(), span: z.object({ startByte: z.number(), endByte: z.number(), startLine: z.number() }).passthrough(),
  variables: z.object({ row: z.record(z.string(), z.unknown()).optional() }).passthrough().optional(),
}).passthrough()) });
const identifier = (value: string): string => `"${value.replaceAll('"', '""')}"`;
const literal = (value: string): string => `'${value.replaceAll("'", "''")}'`;

const { values } = parseArgs({ options: { policy: { type: "string" }, recording: { type: "string", multiple: true },
  output: { type: "string" }, offset: { type: "string", default: "0" }, limit: { type: "string", default: "20" },
  production: { type: "boolean", default: false } } });
if (!values.policy || !values.recording || !values.output) throw new Error("Required: --policy --recording --output");
const native = loadNativeBinding();
const original = readFileSync(values.policy, "utf8");
const source = sourceSchema.parse(JSON.parse(original));
const records = values.recording.map(path => z.object({ commit: z.string(), source_mismatches: z.array(z.string()).max(0),
  files: filesSchema.optional(), events: z.array(z.object({ test: z.string().default("production-files"), check: z.string(), entry: z.string().default("all"),
    files: filesSchema.optional(), findings: z.array(z.unknown()).optional(),
    scope: z.unknown().optional(), production_scope: z.boolean().optional(),
    oracle_config: z.object({ options: z.record(z.string(), z.unknown()).optional() }).passthrough().optional(),
  }).passthrough()).optional(),
}).passthrough().parse(JSON.parse(readFileSync(path, "utf8"))));
if (new Set(records.map(row => row.commit)).size !== 1) throw new Error("Source commits differ");
const events = records.flatMap(record => record.events
  ? record.events.filter(event => event.check === "serverplan_quality").map(event => ({ ...event, files: event.files ?? record.files }))
  : [{ test: "production-files", check: "serverplan_quality", entry: "all", files: record.files,
    findings: [], production_scope: true, scope: undefined, oracle_config: undefined }]);
const offset = Number(values.offset), limit = Number(values.limit);
if (!Number.isSafeInteger(offset) || offset < 0 || !Number.isSafeInteger(limit) || limit < 1) throw new Error("Invalid offset or limit");
const cases = [];
for (const [index, event] of events.slice(offset, offset + limit).entries()) {
  const policy = structuredClone(source);
  if (!values.production) {
    if (!event.production_scope && event.scope) policy.scope = event.scope;
    for (const clause of policy.clauses) if (clause.define?.options) {
      clause.define.config = { ...clause.define.config,
        oracleAnalyzer: !event.production_scope, ...event.oracle_config?.options };
    }
  }
  const options: Record<string, unknown> = {};
  for (const clause of policy.clauses) if (clause.define?.options) {
    for (const [key, option] of Object.entries(clause.define.options)) options[key] = option.default;
    Object.assign(options, clause.define.config);
  }
  const files = filesSchema.parse(event.files);
  const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [],
    files: files.map(file => ({ ...file, language: "python" })) });
  const baseline = resultSchema.parse(JSON.parse(native.CodeRuleDslRuntime.compile(
    JSON.stringify(codeRuleSchema.parse(policy)), "json", capabilitiesJson()).evaluate(input, "report")));
  if (baseline.failures.length) throw new Error(JSON.stringify(baseline.failures));
  if (!values.production && event.findings) {
    const findings = z.array(z.object({ path: z.string(), line: z.number(), code: z.string() })).parse(event.findings);
    const expected = findings.map(row => JSON.stringify([row.path, row.line, row.code])).sort();
    const actual = baseline.violations.map(row => JSON.stringify([row.path, row.span.startLine + 1, row.code])).sort();
    if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error(`Native oracle mismatch: ${event.test}`);
  }
  const capture = structuredClone(policy);
  const schemas: Record<string, Record<string, string[]>> = {};
  const origins = new Map<string, string>();
  capture.clauses = capture.clauses.flatMap(clause => {
    const plan = clause.relational;
    if (!plan) return clause.define ? [clause] : [];
    schemas[clause.id] = Object.fromEntries(Object.entries(plan.inputs).map(([name, relation]) =>
      [name, ["__path", "__start", "__end", ...Object.keys(relation.evidence.project)]]));
    return Object.entries(schemas[clause.id]!).map(([name, columns], index) => {
      const packet = structuredClone(clause);
      packet.id = `${clause.id}-capture-${index}`;
      origins.set(packet.id, clause.id);
      const relation = packet.relational!;
      relation.inputs = { [name]: plan.inputs[name]! };
      relation.stages = [];
      // Bound witnesses and JSON cell size independently. Each diagnostic holds
      // at most 32 original input rows, with a stable per-file packet key.
      relation.query = `WITH packets AS (SELECT *,row_number() OVER(PARTITION BY __path ORDER BY __start,__end) AS rn FROM ${identifier(name)}), chunks AS (SELECT __path,CAST((rn-1)/32 AS INTEGER) AS batch,min(rn) AS first_row,json_group_array(json_array(${columns.map(identifier).join(",")})) AS payload FROM packets GROUP BY __path,CAST((rn-1)/32 AS INTEGER)) SELECT c.__path AS path,a.__start AS start_byte,a.__end AS end_byte,${literal(name)} AS relation,c.batch,c.payload FROM chunks c JOIN packets a ON a.__path=c.__path AND a.rn=c.first_row`;
      relation.key = ["relation", "batch"];
      delete relation.code;
      packet.violation = { code: "REACH_CAPTURE", message: "Reachability SQL input", anchor: "witness.span" };
      return packet;
    });
  });
  const observed = resultSchema.parse(JSON.parse(native.CodeRuleDslRuntime.compile(
    JSON.stringify(codeRuleSchema.parse(capture)), "json", capabilitiesJson()).evaluate(input, "report")));
  if (observed.failures.length) throw new Error(JSON.stringify(observed.failures));
  const rows: Record<string, Record<string, unknown[][]>> = {};
  for (const violation of observed.violations) {
    const clauseId = origins.get(z.string().parse(violation.clauseId));
    if (!clauseId) throw new Error("Unknown capture clause");
    const relation = z.string().parse(violation.variables?.row?.relation);
    const payload = z.string().parse(violation.variables?.row?.payload);
    ((rows[clauseId] ??= {})[relation] ??= []).push(...z.array(z.array(z.unknown())).parse(JSON.parse(payload)));
  }
  cases.push({ name: `${offset + index}:${event.test}`, files, options, schemas, rows,
    expected: baseline.violations.flatMap(violation => violation.variables?.row ? [{ ...violation.variables.row }] : []),
    nonRelationalExpected: baseline.violations.filter(violation => !violation.variables?.row),
    upstream: event.findings, nativeOracleParity: values.production ? null : true, production: values.production });
}
writeFileSync(values.output, JSON.stringify({ format: "rerail.reachability-capture/v1", commit: records[0]!.commit,
  nativeSha256: createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex"),
  policySha256: createHash("sha256").update(original).digest("hex"), policy: source, totalCases: events.length, cases }) + "\n");
console.log(JSON.stringify({ cases: cases.length, totalCases: events.length, positive: cases.filter(row => row.expected.length).length,
  inputRows: cases.reduce((sum, row) => sum + Object.values(row.rows).reduce((n, inputs) => n + Object.values(inputs).reduce((m, rows) => m + rows.length, 0), 0), 0) }));
