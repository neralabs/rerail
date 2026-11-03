import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, loadNativeBinding } from "../../../src/engine/index.js";
import { capabilitiesJson } from "../../../conformance/engine.js";
import { fixturesPath } from "../../support/oracles.js";

// A Rust unit build includes cfg(test) code. This gate deliberately loads the
// production N-API addon, which previously rejected ten established policies.
it("accepts all sixty established policies through the production compiler", () => {
  const manifest = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batches.json"), "utf8"));
  const checks: string[] = manifest.batches.filter((batch: { id: number }) => batch.id <= 6).flatMap((batch: { checks: string[] }) => batch.checks);
  expect(checks).toHaveLength(60);
  const failures = checks.flatMap(check => {
    try {
      const policy = codeRuleSchema.parse(JSON.parse(readFileSync(`packs/data-layer/candidates/rules/data-layer-${check.replaceAll("_", "-")}.json`, "utf8")));
      loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilitiesJson());
      return [];
    } catch (error) {
      return [{ check, error: String(error) }];
    }
  });
  expect(failures).toEqual([]);
}, 60_000);
