import { policyPath } from "./policies.js";
/** Persistent native backend for the unchanged upstream pytest assertions. */
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { createInterface } from "node:readline";
import { z } from "zod";
import { codeRuleSchema, loadNativeBinding, RESULT_PAGE_BYTES, type NativeCompiledCodeRuleDsl } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";
import { orderFactFindings, type FactFinding } from "./fact-api.js";
import { cachedDiscoveryPolicy } from "./cached-discovery.js";
import { replayExecution } from "./execution.js";

const requestSchema = z.object({
  check: z.string(), entry: z.string(), files: z.array(z.object({ path: z.string(), source: z.string() })),
  options: z.record(z.string(), z.unknown()).default({}),
  contract: z.string().optional(), parameters: z.record(z.string(), z.unknown()).optional(),
  mode: z.enum(["snapshot", "workspace"]),
}).strict();
const authoredSchema = z.object({ clauses: z.array(z.object({ define: z.object({
  options: z.record(z.string(), z.unknown()).optional(), config: z.record(z.string(), z.unknown()).optional(),
}).passthrough().optional() }).passthrough()) }).passthrough();
const pageSchema = z.object({ done: z.boolean(), cursor: z.number(), violations: z.array(z.unknown()), failures: z.array(z.unknown()) });
const syntaxFinding = z.object({ code: z.literal("NATIVE-SYNTAX"), path: z.string(), variables: z.record(z.string(), z.unknown()) }).passthrough();
process.env.RERAIL_CODE_RULE_PROFILE = "1";
let execution = replayExecution();
const compiled = new Map<string, NativeCompiledCodeRuleDsl>();
const contracts = process.env.RERAIL_NATIVE_CONTRACTS === undefined ? {} : JSON.parse(readFileSync(process.env.RERAIL_NATIVE_CONTRACTS, "utf8"));
try {
  for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
    try {
      const raw: unknown = JSON.parse(line);
      if (z.object({ reset: z.literal(true) }).strict().safeParse(raw).success) {
        execution.close(); execution = replayExecution();
        process.stdout.write(JSON.stringify({ reset: true }) + "\n"); continue;
      }
      const request = requestSchema.parse(raw);
      const authored = authoredSchema.parse(request.contract === undefined
        ? JSON.parse(readFileSync(policyPath(process.env.RERAIL_NATIVE_POLICY_DIRECTORY ?? "packs/data-layer/candidates/rules", `data-layer-${request.check.replaceAll("_", "-")}`), "utf8"))
        : ["lazy-analyzed", "cached-policy-comparison", "memoized-policy-comparison"].includes(request.contract) ? JSON.parse(readFileSync(resolve(dirname(process.env.RERAIL_NATIVE_POLICY_DIRECTORY ?? "packs/data-layer/candidates/rules"), "facts", `${request.contract}.json`), "utf8")) : request.contract === "cached-discovery" ? cachedDiscoveryPolicy(request.parameters?.module_name as string | undefined) : structuredClone(contracts[request.contract]));
      for (const clause of authored.clauses) if (clause.define?.options !== undefined) {
        const values = { entryPoint: request.entry, ...request.options,
          ...(request.parameters === undefined ? {} : { parameters: JSON.stringify(request.parameters) }) };
        clause.define.config = { ...clause.define.config, ...Object.fromEntries(Object.entries(values)
          .filter(([name]) => Object.hasOwn(clause.define!.options!, name))) };
      }
      const policy = codeRuleSchema.parse(authored);
      policy.scope = { include: ["**/*"], exclude: [] };
      const key = JSON.stringify(policy);
      let rule = compiled.get(key);
      if (rule === undefined) {
        rule = loadNativeBinding().CodeRuleDslRuntime.compile(key, "json", capabilitiesJson()); compiled.set(key, rule);
      }
      const files = request.files.map(file => ({ ...file, language: file.path.endsWith(".py") ? "python" : "text" }));
      let result: { violations: unknown[]; failures: unknown[]; telemetry?: unknown };
      if (request.mode === "snapshot") result = JSON.parse(execution.evaluate(rule,
        JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files, facts: [] }), "report"));
      else {
        const workspace = execution.createWorkspace(); result = { violations: [], failures: [] };
        try {
          workspace.addCodeRule(rule);
          result.telemetry = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files, removed: [], quiet: true })));
          for (let cursor = 0; ;) {
            const page = pageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
            result.violations.push(...page.violations); result.failures.push(...page.failures);
            if (page.done) break;
            if (page.cursor <= cursor) throw new Error("Native pytest backend cursor did not advance");
            cursor = page.cursor;
          }
        } finally { workspace.close(); }
      }
      if (request.contract?.endsWith("policy-comparison") || request.contract==="lazy-analyzed" || request.check==="lazy_architecture")
        result.violations=orderFactFindings(request.contract,result.violations as FactFinding[], request.files.map(file=>file.path));
      if (result.failures.length) throw new Error(JSON.stringify(result.failures));
      const invalid = result.violations.flatMap(row => {
        const parsed = syntaxFinding.safeParse(row); return parsed.success ? [parsed.data] : [];
      });
      if (invalid.length) {
        const documents = invalid.map(row => {
          const file = request.files.find(file => file.path === row.path);
          if (file === undefined) throw new Error(`Missing syntax input: ${row.path}`);
          return { source: file.source, requests: [{ start: 0, end: Buffer.byteLength(file.source), kind: "module", fields: ["syntax_error"] }] };
        });
        const response = z.object({ documents: z.array(z.array(z.object({ syntax_error: z.string() }))) })
          .parse(JSON.parse(loadNativeBinding().CodeRuleDslRuntime.pythonAstBatch(JSON.stringify(documents))));
        const errors = new Map(invalid.map((row, index) => [row.path, JSON.parse(response.documents[index]![0]!.syntax_error)]));
        result.violations = result.violations.map(row => {
          const parsed = syntaxFinding.safeParse(row);
          return parsed.success ? { ...parsed.data, variables: { ...parsed.data.variables, syntax_error: errors.get(parsed.data.path) } } : row;
        });
      }
      process.stdout.write(JSON.stringify({ result }) + "\n");
    } catch (error) { process.stdout.write(JSON.stringify({ error: String(error) }) + "\n"); }
  }
} finally { execution.close(); }
