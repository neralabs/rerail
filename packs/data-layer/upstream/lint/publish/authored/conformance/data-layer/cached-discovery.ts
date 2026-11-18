/** Inspect the production cached-policy relations against original discovery results. */
import { readFileSync, writeFileSync } from "node:fs";
import { isAbsolute } from "node:path";
import { parseArgs } from "node:util";
import { z } from "zod";
import { codeRuleSchema, loadNativeBinding, RESULT_PAGE_BYTES, type NativeCompiledCodeRuleDsl } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";
import { replayExecution } from "./execution.js";

const functionSchema = z.object({
  function: z.string(), path: z.string(), line: z.number(), cache_args: z.array(z.string()),
  cache_annotations: z.array(z.tuple([z.string(), z.string()])), body_hash: z.string(), signature_hash: z.string(),
  cache_members: z.array(z.tuple([z.string(), z.string()])), errors: z.array(z.string()),
});
const eventSchema = z.object({ test: z.string(), helper: z.literal("scan_cached_pure_lazy_source"), path: z.string(),
  source: z.string(), module_name: z.string(), functions: z.array(functionSchema).optional(),
  exception: z.object({ type: z.string() }).passthrough().optional() });
const resultSchema = z.object({ violations: z.array(z.object({ variables: z.object({ row: z.object({ payload: z.string() }) }) })),
  failures: z.array(z.unknown()) });

// Only the terminal projection changes. The producer demands, fingerprints,
// aliases and error relations are the production rule's actual implementation.
const projection = `SELECT n.__path path,n.__start start_byte,n.__end end_byte,f.function identity,
  json_object('function',f.function,'path',f.__path,'line',n.line+1,
    'body_hash',f.body_sha256,'signature_hash',f.signature_sha256,
    'cache_args',(SELECT json_group_array(name) FROM (SELECT name FROM cache_arguments a WHERE a.__path=f.__path AND a.fn=f.id ORDER BY ordinal)),
    'cache_annotations',(SELECT json_group_array(json_array(name,unparse)) FROM (SELECT p.name,v.unparse FROM parameters p JOIN annotation_values v ON v.__path=p.__path AND v.id=p.annotation WHERE p.__path=f.__path AND p.fn=f.id AND p.name IN (SELECT name FROM cache_arguments a WHERE a.__path=f.__path AND a.fn=f.id) ORDER BY p.name)),
    'cache_members',(SELECT json_group_array(json_array(member,mode)) FROM (SELECT member,mode FROM cache_members m WHERE m.__path=f.__path AND m.fn=f.id ORDER BY member,mode)),
    'errors',(SELECT json_group_array(error) FROM (SELECT error FROM cached_errors e WHERE e.__path=f.__path AND e.fn=f.id UNION SELECT error FROM extra_cached_errors e WHERE e.__path=f.__path AND e.fn=f.id ORDER BY error))) payload
  FROM actual_functions f JOIN nodes n ON n.__path=f.__path AND n.id=f.id`;

export function cachedDiscoveryPolicy(moduleName?: string) {
  const policy = codeRuleSchema.parse(JSON.parse(readFileSync("packs/data-layer/rules/data-layer-cached-pure-lazy-allowlist.json", "utf8")));
  policy.scope = { include: ["**/*.py"], exclude: [] };
  policy.clauses = policy.clauses.filter(clause => clause.define !== undefined || clause.id === "cached-policy");
  const plan = policy.clauses.find(clause => clause.id === "cached-policy")!.relational!;
  if (moduleName !== undefined) plan.stages!.find(stage => stage.name === "module_names")!.query =
    `SELECT __path,'${moduleName.replaceAll("'", "''")}' module FROM nodes WHERE kind='module'`;
  policy.clauses.push({ id: "native-syntax", forbid: { ast: { language: "python", syntax: "invalid", rule: { kind: "module" } } },
    violation: { code: "NATIVE-SYNTAX", message: "Invalid Python syntax", anchor: "witness.span" } });
  plan.query = projection; plan.key = ["identity"]; delete plan.code; delete plan.message;
  return policy;
}

function ordered(functions: z.infer<typeof functionSchema>[]): string[] {
  // Parse first to compare the same complete field set and field order.
  return functions.map(value => JSON.stringify(functionSchema.parse(value))).sort();
}

export async function replayCachedDiscovery(raw: unknown[]) {
  const events = raw.map(event => eventSchema.parse(event));
  const execution = replayExecution();
  const compiled = new Map<string, NativeCompiledCodeRuleDsl>();
  const report = { tests: [] as string[], positive: 0, clean: 0, syntaxContracts: [] as string[],
    failures: [] as unknown[], differences: [] as { test: string; path: "snapshot" | "workspace"; expected: string[]; actual: string[] }[] };
  try {
    for (const event of events) {
      // The original API raises SyntaxError before discovery. The linter API
      // has a different syntax contract; do not claim an empty result matches it.
      if (event.exception !== undefined) { report.syntaxContracts.push(event.test); continue; }
      if (event.functions === undefined) throw new Error(`Missing discovery result: ${event.test}`);
      report.tests.push(event.test);
      if (event.functions.length) report.positive++; else report.clean++;
      let rule = compiled.get(event.module_name);
      if (rule === undefined) {
        const policy = cachedDiscoveryPolicy(event.module_name);
        rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilitiesJson());
        compiled.set(event.module_name, rule);
      }
      // Absolute paths are oracle identities. Use one stable relative input
      // identity and restore the original path only for field comparison.
      const path = isAbsolute(event.path) ? "input.py" : event.path;
      const files = [{ path, source: event.source, language: "python" }];
      const snapshot = resultSchema.parse(JSON.parse(execution.evaluate(rule, JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] }), "report")));
      const workspace = execution.createWorkspace();
      const resident: z.infer<typeof resultSchema> = { violations: [], failures: [] };
      try {
        workspace.addCodeRule(rule);
        await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [], quiet: true }));
        for (let cursor = 0; ;) {
          const page = resultSchema.extend({ done: z.boolean(), cursor: z.number() }).parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
          resident.violations.push(...page.violations); resident.failures.push(...page.failures);
          if (page.done) break;
          if (page.cursor <= cursor) throw new Error("Discovery page did not advance");
          cursor = page.cursor;
        }
      } finally { workspace.close(); }
      const expected = ordered(event.functions);
      for (const [name, result] of [["snapshot", snapshot], ["workspace", resident]] as const) {
        report.failures.push(...result.failures);
        const actual = ordered(result.violations.map(finding => ({ ...JSON.parse(finding.variables.row.payload), path: event.path })));
        if (JSON.stringify(expected) !== JSON.stringify(actual)) report.differences.push({ test: event.test, path: name, expected, actual });
      }
    }
  } finally { execution.close(); }
  return report;
}

if (process.argv[1]?.endsWith("/cached-discovery.ts")) {
  const { values } = parseArgs({ options: { recording: { type: "string" }, output: { type: "string" } } });
  if (!values.recording || !values.output) throw new Error("Usage: cached-discovery.ts --recording <json> --output <json>");
  const recording = JSON.parse(readFileSync(values.recording, "utf8"));
  const report = await replayCachedDiscovery(recording.helper_events);
  writeFileSync(values.output, JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify({ positive: report.positive, clean: report.clean, differences: report.differences.length, failures: report.failures.length }));
  if (report.failures.length || report.differences.length) process.exitCode = 1;
}
