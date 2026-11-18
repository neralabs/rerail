import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding, RESULT_PAGE_BYTES } from "../../../src/engine/index.js";
import { capabilitiesJson } from "../../../conformance/engine.js";
import { orderFactFindings } from "../../../conformance/data-layer/fact-api.js";
import { replayExecution } from "../../../conformance/data-layer/execution.js";

const oracle = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-08-fact-api.json", "utf8")) as {
  cases: { id: string; contract: string; parameters: unknown; expected: unknown }[];
};
for (const mode of ["snapshot", "workspace"] as const) {
  it.each(oracle.cases)(`${mode} executes supplied fact batch $id without reading source`, async (test: { contract: string; parameters: unknown; expected: unknown }) => {
    const policy = JSON.parse(readFileSync(`packs/data-layer/candidates/facts/${test.contract}.json`, "utf8"));
    policy.clauses[0].define.config = { parameters: JSON.stringify(test.parameters) };
    const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(policy)), "json", capabilitiesJson());
    const execution = replayExecution();
    let result: { violations: { code: string; variables: { row: { payload: string } } }[]; failures: unknown[] };
    try {
      if (mode === "snapshot") result = JSON.parse(execution.evaluate(rule, JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [] }), "report"));
      else {
        const workspace = execution.createWorkspace(); result = { violations: [], failures: [] };
        try {
          workspace.addCodeRule(rule);
          await workspace.applyAsync(JSON.stringify({ revision: 1, files: [], removed: [], quiet: true }));
          for (let cursor = 0; ;) {
            const page = JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES));
            result.violations.push(...page.violations); result.failures.push(...page.failures);
            if (page.done) break;
            expect(page.cursor).toBeGreaterThan(cursor); cursor = page.cursor;
          }
        } finally { workspace.close(); }
      }
      expect(result.failures).toEqual([]);
      result.violations = orderFactFindings(test.contract,result.violations as (typeof result.violations[number] & {path:string})[]);
      if (test.contract === "lazy-analyzed") {
        const violations = result.violations.map(row => JSON.parse(row.variables.row.payload));
        const counts = Object.fromEntries(Array.from({ length: 18 }, (_, i) => [`LA-${String(i + 1).padStart(3, "0")}`, 0]));
        for (const row of violations) counts[row.rule] = (counts[row.rule] ?? 0) + 1;
        expect([counts, violations]).toEqual(test.expected);
      } else expect(result.violations.map(row => row.code)).toEqual(test.expected);
    } finally { execution.close(); }
  });
}
