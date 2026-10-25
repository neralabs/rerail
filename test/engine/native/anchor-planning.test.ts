import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { loadNativeBinding } from "../../../src/engine/index.js";

const native = loadNativeBinding();
const capabilities = readFileSync(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url), "utf8");
const policy = readFileSync(new URL("../../../packs/data-layer/candidates/rules/data-layer-sync-async-signatures.json", import.meta.url), "utf8");

// Reduced from the full repository's generated_facades.py failure. No function
// has an async name, so this policy has no signature pair to report.
const file = {
  path: "impl/common/l2100_sql_grammar/rust_bridge/generated_facades.py",
  language: "python",
  source: Array.from({ length: 1000 }, (_, i) =>
    `def method_${i}(connection: SqlTransaction, transaction: SqlConnection):\n    return connection, transaction\n`,
  ).join("\n"),
};

it("snapshot planning rejects absent async names before utility work exhausts its allowance", () => {
  const rule = native.CodeRuleDslRuntime.compile(policy, "json", capabilities);
  const result = JSON.parse(rule.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1", revision: 1, files: [file],
  }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toEqual([]);
}, 60_000);

it("workspace planning rejects absent async names before utility work exhausts its allowance", async () => {
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(native.CodeRuleDslRuntime.compile(policy, "json", capabilities));
    const result = JSON.parse(await workspace.applyAsync(JSON.stringify({ revision: 1, files: [file], removed: [] })));
    expect(result.evaluation.failures).toEqual([]);
    expect(result.evaluation.violations).toEqual([]);
  } finally {
    workspace.close();
  }
}, 60_000);
