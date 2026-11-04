/** Persistent native backend for the unchanged upstream pytest assertions. */
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { z } from "zod";
import { codeRuleSchema, loadNativeBinding, RESULT_PAGE_BYTES, type NativeCompiledCodeRuleDsl } from "../../src/engine/index.js";
import { capabilitiesJson } from "../engine.js";
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
process.env.RERAIL_CODE_RULE_PROFILE = "1";
const execution = replayExecution();
const compiled = new Map<string, NativeCompiledCodeRuleDsl>();
const contracts = process.env.RERAIL_NATIVE_CONTRACTS === undefined ? {} : JSON.parse(readFileSync(process.env.RERAIL_NATIVE_CONTRACTS, "utf8"));
try {
  for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
    try {
      const request = requestSchema.parse(JSON.parse(line));
      const authored = authoredSchema.parse(request.contract === undefined
        ? JSON.parse(readFileSync(`${process.env.RERAIL_NATIVE_POLICY_DIRECTORY ?? "packs/data-layer/candidates/rules"}/data-layer-${request.check.replaceAll("_", "-")}.json`, "utf8"))
        : request.contract === "cached-discovery" ? cachedDiscoveryPolicy(request.parameters?.module_name as string | undefined) : structuredClone(contracts[request.contract]));
      for (const clause of authored.clauses) if (clause.define?.options !== undefined) {
        const values = { oracleAnalyzer: true, entryPoint: request.entry, ...request.options,
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
      const files = request.files.map(file => ({ ...file, language: "python" }));
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
      if (result.failures.length) throw new Error(JSON.stringify(result.failures));
      process.stdout.write(JSON.stringify({ result }) + "\n");
    } catch (error) { process.stdout.write(JSON.stringify({ error: String(error) }) + "\n"); }
  }
} finally { execution.close(); }
