import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";
import { capabilitiesJson } from "../../../conformance/engine.js";
const audit = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-09-audit-20261105.json", "utf8"));
for (const spec of audit.publicDefaultEvaluations.filter((r: { id: string }) => r.id !== "composition-default")) {
  it(`preserves upstream ${spec.id} through the shipped scope and defaults`, () => {
    const authored = codeRuleSchema.parse(JSON.parse(readFileSync(`packs/data-layer/rules/data-layer-${spec.check}.json`, "utf8")));
    const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(authored), "json", capabilitiesJson());
    const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [{ path: spec.path, source: spec.source, language: "python" }] }), "report"));
    expect(result.failures).toEqual([]);
    expect(result.violations.map((r: { code: string; message: string; span: { startLine: number } }) => ({ code: r.code, message: r.message, line: r.span.startLine + 1 })))
      .toEqual(spec.expected.map((r: { code: string; message: string; line: number }) => ({ code: r.code, message: r.message, line: r.line })));
  });
}
