/** Replay the unchanged data-layer tests' recorded inputs through the native engine. */
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { createHash } from "node:crypto";
import { parseArgs } from "node:util";
import { z } from "zod";
import { codeRuleSchema, loadNativeBinding, resolveNativeBindingPath, RESULT_PAGE_BYTES, type NativeCompiledCodeRuleDsl } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";

const findingSchema = z.object({ path: z.string(), line: z.number(), column: z.number().nullable(), code: z.string(), message: z.string(), tag: z.string().nullable().optional() });
const eventSchema = z.object({
  test: z.string(), call: z.number(), check: z.string(), entry: z.string(), upstream_passed: z.boolean(),
  retained_reason: z.string().optional(), oracle_ms: z.number().optional(),
  production_scope: z.boolean().optional(), path_skip_override: z.boolean().optional(),
  compare_clause: z.boolean().optional(),
  oracle_config: z.object({ categories: z.record(z.string(), z.array(z.string())).optional(), source_gate: z.boolean().optional(), options: z.record(z.string(), z.unknown()).optional() }).optional(),
  scope: z.object({ include: z.array(z.string()), exclude: z.array(z.string()) }).optional(),
  files: z.array(z.object({ path: z.string(), source: z.string() })).optional(),
  findings: z.array(findingSchema).optional(),
});
const recordingSchema = z.object({
  format: z.literal("rerail.data-layer-recording/v1"), commit: z.string(), checks: z.array(z.string()),
  source_mismatches: z.array(z.string()), pytest_exit_code: z.number(),
  files: z.array(z.object({ path: z.string(), source: z.string() })).optional(),
  collected_tests: z.number(), uncaptured_tests: z.array(z.string()), events: z.array(eventSchema),
});
const resultSchema = z.object({
  violations: z.array(z.object({ code: z.string(), path: z.string(), message: z.string(), clauseId: z.string(),
    span: z.object({ startByte: z.number(), startLine: z.number(), startColumn: z.number() }) })),
  failures: z.array(z.object({ code: z.string(), message: z.string(), path: z.string().optional() })),
});
const pageSchema = resultSchema.extend({ done: z.boolean(), cursor: z.number() });
// Configure authored options before the main schema expands their references.
const configurableSchema = z.object({ clauses: z.array(z.object({
  define: z.object({ options: z.record(z.string(), z.unknown()).optional(),
    config: z.record(z.string(), z.unknown()).optional() }).passthrough().optional(),
}).passthrough()) }).passthrough();
type Event = z.infer<typeof eventSchema>;
type Result = z.infer<typeof resultSchema>;
interface ReplayRow {
  test: string; call: number; check: string; expected: number; actual: number; positive: boolean;
  falsePositives: string[]; falseNegatives: string[]; executionPathDifferences: string[];
  engineFailures: Result["failures"]; statelessFailures: Result["failures"];
  statelessMs: number; workspaceMs: number; oracleMs: number | undefined;
}

function subtract(left: string[], right: string[]): string[] {
  const counts = new Map<string, number>();
  for (const key of right) counts.set(key, (counts.get(key) ?? 0) + 1);
  return left.filter(key => {
    const count = counts.get(key) ?? 0;
    if (count === 0) return true;
    counts.set(key, count - 1);
    return false;
  });
}

async function workspaceEvaluation(compiled: NativeCompiledCodeRuleDsl, event: Event): Promise<Result> {
  const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
  try {
    workspace.addCodeRule(compiled);
    await workspace.applyAsync(JSON.stringify({ revision: 1, files: event.files!.map(file => ({ ...file, language: file.path.endsWith(".rs") ? "rust" : "python" })), removed: [], quiet: true }));
    const result: Result = { violations: [], failures: [] };
    for (let cursor = 0; ;) {
      const page = pageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
      result.violations.push(...page.violations);
      result.failures.push(...page.failures);
      if (page.done) return result;
      if (page.cursor <= cursor) throw new Error("Native result cursor did not advance");
      cursor = page.cursor;
    }
  } finally { workspace.close(); }
}

export async function replay(recordingPath: string | string[], outputPath: string, policies = "packs/data-layer/candidates/rules"): Promise<boolean> {
  const paths = typeof recordingPath === "string" ? [recordingPath] : recordingPath;
  if (paths.length === 0) throw new Error("At least one recording is required");
  const inputs = paths.map(path => recordingSchema.parse(JSON.parse(readFileSync(path, "utf8"))));
  const first = inputs[0]!;
  for (const input of inputs) {
    if (input.pytest_exit_code !== 0 || input.source_mismatches.length !== 0) throw new Error("Upstream tests or pinned source verification failed");
    if (input.commit !== first.commit) throw new Error("Combined recordings must use the same source commit");
  }
  const recording = { ...first, files: undefined,
    checks: [...new Set(inputs.flatMap(input => input.checks))],
    collected_tests: inputs.reduce((sum, input) => sum + input.collected_tests, 0),
    uncaptured_tests: inputs.flatMap(input => input.uncaptured_tests),
    events: inputs.flatMap(input => input.events.map(event => ({ ...event, files: event.files ?? input.files }))),
  };
  const compiled = new Map<string, NativeCompiledCodeRuleDsl>();
  const validated = new Map<string, z.infer<typeof codeRuleSchema>>();
  const rows: ReplayRow[] = [];
  const started = performance.now();
  let compilationMs = 0;
  for (const event of recording.events) {
    if (event.retained_reason !== undefined || !event.upstream_passed) continue;
    event.files ??= recording.files;
    if (event.files === undefined || event.scope === undefined || event.findings === undefined) throw new Error(`Incomplete event ${event.test}`);
    const authored = configurableSchema.parse(JSON.parse(readFileSync(resolve(policies, `data-layer-${event.check.replaceAll("_", "-")}.json`), "utf8")));
    for (const clause of authored.clauses) {
      if (clause.define?.options?.oracleAnalyzer !== undefined) {
        clause.define.config = { ...clause.define.config, oracleAnalyzer: !event.production_scope };
      }
      if (clause.define?.options?.sourceGate !== undefined && event.oracle_config?.source_gate !== undefined) {
        clause.define.config = { ...clause.define.config, sourceGate: event.oracle_config.source_gate };
      }
      if (event.oracle_config?.categories !== undefined && clause.define?.options !== undefined) {
        const assigned = new Set<string>();
        const categories: Record<string, string[]> = {};
        for (const [category, names] of Object.entries(event.oracle_config.categories)) {
          categories[category] = names.filter(name => !assigned.has(name));
          for (const name of names) assigned.add(name);
        }
        clause.define.config ??= {};
        for (const option of Object.keys(clause.define.options)) {
          if (option.startsWith("category-")) clause.define.config[option] = categories[option.slice("category-".length)] ?? [];
        }
      }
    }
    for (const clause of authored.clauses) {
      if (clause.define?.options !== undefined && event.oracle_config?.options !== undefined) {
        clause.define.config = { ...clause.define.config, ...event.oracle_config.options };
      }
    }
    const authoredKey = JSON.stringify(authored);
    let validatedPolicy = validated.get(authoredKey);
    if (validatedPolicy === undefined) {
      validatedPolicy = codeRuleSchema.parse(authored);
      validated.set(authoredKey, validatedPolicy);
    }
    const policy = structuredClone(validatedPolicy);
    // Explicit-file tests bypass the CLI's default roots, exactly as their original entry points do.
    if (!event.production_scope) policy.scope = event.scope.include.length === 0 ? { include: ["**/*"], exclude: ["**/*"] } : event.scope;
    if (event.check === "no_blanket_grammar_bypass" && !event.production_scope) {
      policy.clauses = policy.clauses.filter(clause => event.entry === "check_no_public_grammar_methods" ? clause.id === "public-methods" : clause.id !== "public-methods");
    }
    if (event.check === "type_safety_patterns" && !event.production_scope && ["check_function_signatures", "check_file_for_patterns"].includes(event.entry)) {
      policy.clauses = policy.clauses.filter(clause => event.entry === "check_function_signatures"
        ? clause.id.startsWith("signature-") : clause.id.startsWith("text-"));
    }
    if (event.check === "transaction_closures" && event.entry.endsWith(".visit")) {
      const prefix = event.entry === "TransactionClosureLinter.visit" ? "inline-" : "named-";
      policy.clauses = policy.clauses.filter(clause => clause.define !== undefined || clause.id.startsWith(prefix));
    }
    if (event.check === "sql_grammar_escape_hatches" && event.entry.startsWith("scan_usages")) {
      policy.clauses = policy.clauses.filter(clause => clause.define !== undefined || clause.id.startsWith("usage-"));
    }
    if (event.path_skip_override) {
      for (const clause of policy.clauses) if (clause.forbid !== undefined && "regex" in clause.forbid) delete clause.forbid.regex.paths;
    }
    const key = JSON.stringify(policy);
    let rule = compiled.get(key);
    if (rule === undefined) {
      const before = performance.now();
      rule = loadNativeBinding().CodeRuleDslRuntime.compile(key, "json", capabilitiesJson());
      compilationMs += performance.now() - before;
      compiled.set(key, rule);
    }
    const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: event.files.map(file => ({ ...file, language: file.path.endsWith(".rs") ? "rust" : "python" })) });
    const before = performance.now();
    const stateless = resultSchema.parse(JSON.parse(rule.evaluate(input, "report")));
    const statelessMs = performance.now() - before;
    const workspaceStarted = performance.now();
    const result = await workspaceEvaluation(rule, event);
    const workspaceMs = performance.now() - workspaceStarted;
    const columns = event.check === "backend_type_strings";
    const expected = event.findings.map(finding => {
      if (event.compare_clause && finding.tag == null) throw new Error(`Missing diagnostic tag: ${event.test}`);
      return JSON.stringify([finding.path, finding.line, finding.code, ...(columns ? [finding.column] : []), ...(event.compare_clause ? [finding.tag] : [])]);
    });
    const actual = (result: Result): string[] => result.violations.map(finding => {
      const file = event.files!.find(file => file.path === finding.path)!;
      const bytes = Buffer.from(file.source);
      const prefix = bytes.subarray(finding.span.startByte - finding.span.startColumn, finding.span.startByte).toString("utf8");
      const category = event.check === "connection_caches" ? finding.clauseId.replace(/-file-\d+$/, "") : finding.clauseId;
      return JSON.stringify([finding.path, finding.span.startLine + 1, finding.code, ...(columns ? [[...prefix].length + 1] : []), ...(event.compare_clause ? [category] : [])]);
    });
    const observed = actual(result);
    rows.push({ test: event.test, call: event.call, check: event.check,
      expected: expected.length, actual: observed.length, positive: expected.length > 0,
      falsePositives: subtract(observed, expected), falseNegatives: subtract(expected, observed),
      engineFailures: result.failures, statelessFailures: stateless.failures,
      executionPathDifferences: [...subtract(actual(stateless), observed), ...subtract(observed, actual(stateless))],
      statelessMs, workspaceMs, oracleMs: event.oracle_ms,
    });
  }
  const coverage = recording.checks.map(check => ({ check,
    positive: rows.filter(row => row.check === check && row.positive).length,
    clean: rows.filter(row => row.check === check && !row.positive).length,
  }));
  const missingCoverage = coverage.filter(row => row.positive === 0 || row.clean === 0);
  const report = { coverage, missingCoverage, nativeSha256: createHash("sha256").update(readFileSync(resolveNativeBindingPath())).digest("hex"), dimensions: ["multiplicity", "relative path", "line", "diagnostic code", "backend comparison column", "diagnostic category where recorded"],
    format: "rerail.data-layer-parity/v1", commit: recording.commit, recordings: paths,
    collectedTests: recording.collected_tests, replayedCalls: rows.length,
    positiveCalls: rows.filter(row => row.positive).length, cleanCalls: rows.filter(row => !row.positive).length,
    retainedCalls: recording.events.filter(event => event.retained_reason !== undefined).map(event => ({ test: event.test, reason: event.retained_reason })),
    uncapturedTests: recording.uncaptured_tests,
    falsePositives: rows.reduce((sum, row) => sum + row.falsePositives.length, 0),
    falseNegatives: rows.reduce((sum, row) => sum + row.falseNegatives.length, 0),
    engineFailures: rows.reduce((sum, row) => sum + row.engineFailures.length + row.statelessFailures.length, 0),
    executionPathDifferences: rows.reduce((sum, row) => sum + row.executionPathDifferences.length, 0),
    compilationMs, elapsedMs: performance.now() - started, rows };
  writeFileSync(outputPath, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ ...report, rows: undefined, retainedCalls: report.retainedCalls.length, uncapturedTests: report.uncapturedTests.length }, null, 2));
  return report.falsePositives + report.falseNegatives + report.engineFailures + report.executionPathDifferences + missingCoverage.length === 0;
}

if (process.argv[1]?.endsWith("/replay.ts")) {
  const { values } = parseArgs({ options: { recording: { type: "string", multiple: true }, output: { type: "string" }, policies: { type: "string" } } });
  if (values.recording === undefined || values.output === undefined) throw new Error("Usage: replay.ts --recording <json> --output <json> [--policies <directory>]");
  if (!await replay(values.recording, values.output, values.policies)) process.exitCode = 1;
}
