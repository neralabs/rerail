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

it("verifies detached signatures without treating malformed signatures as runtime failures", () => {
  // RFC 8032 section 7.1, empty-message signature. No signing key is used.
  const publicKey = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
  const signature = Buffer.from("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b", "hex").toString("base64");
  for (const [key, message, signed, expected] of [[publicKey, "", signature, 0], [publicKey, "changed", signature, 1], ["invalid", "", "invalid", 1]] as const) {
    const literal = (value: string) => `'${value.replaceAll("'", "''")}'`;
    const policy = {
      format: "rerail.policy/v1", id: "signature-contract", revision: 1, mode: "block",
      scope: { include: ["**/*.py"], exclude: [] }, clauses: [{ id: "signature", relational: {
        inputs: { files: { evidence: { as: "file", query: { ast: { language: "python", rule: { kind: "module" } } }, project: { path: "file.path" } } } },
        query: `SELECT __path AS path,__start AS start_byte,__end AS end_byte FROM files WHERE NOT verify_ed25519(${literal(key)},${literal(message)},${literal(signed)})`,
        key: ["path"],
      }, violation: { code: "invalid-signature", message: "Invalid signature", anchor: "witness.span" } }],
    };
    const compiled = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(policy)), "json", capabilitiesJson());
    const result = JSON.parse(compiled.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 1, facts: [], files: [{ path: "example.py", language: "python", source: "" }] }), "report"));
    expect(result.failures).toEqual([]);
    expect(result.violations).toHaveLength(expected);
  }
});

it("retains declared diagnostic anchors on relational input rows", () => {
  const policy = {
    format: "rerail.policy/v1", id: "anchor-contract", revision: 1, mode: "block",
    scope: { include: ["**/*.py"], exclude: [] }, clauses: [{ id: "file-diagnostic", relational: {
      inputs: { functions: { evidence: { as: "function", query: { ast: { language: "python", rule: { kind: "function_definition" } } }, project: { path: "file.path" }, anchor: "file.start" } } },
      query: "SELECT __path AS path,__start AS start_byte,__end AS end_byte FROM functions", key: ["path"],
    }, violation: { code: "file-diagnostic", message: "File diagnostic", anchor: "witness.span" } }],
  };
  const compiled = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(policy)), "json", capabilitiesJson());
  const result = JSON.parse(compiled.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 1, facts: [], files: [{ path: "example.py", language: "python", source: "\n\ndef f():\n    pass\n" }] }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations).toMatchObject([{ span: { startByte: 0, startLine: 0 } }]);
});
