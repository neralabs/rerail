import { policyPath } from "../../../conformance/data-layer/policies.js";
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { codeRuleSchema, encodeCodeRuleWorkspaceRequest, loadNativeBinding, RESULT_PAGE_BYTES } from "../../../src/engine/index.js";
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
      const policy = codeRuleSchema.parse(JSON.parse(readFileSync(policyPath("packs/data-layer/candidates/rules", `data-layer-${check.replaceAll("_", "-")}`), "utf8")));
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

it.each([false, true])("hydrates only admitted repository source (syntax: %s)", async syntax => {
  const native = loadNativeBinding();
  const input = (pattern: string) => ({ evidence: { as: "file", query: { ast: { language: "python", rule: { kind: "module", regex: pattern } } }, project: { path: "file.path" } } });
  const policy = {
    format: "rerail.policy/v1", id: "admitted-source-contract", revision: 1, mode: "block",
    scope: { include: ["**/*.py"], exclude: [] }, clauses: [{ id: "admitted-source", relational: {
      inputs: { gate: input("^selected") }, requireNonemptyInput: "gate",
      ...(syntax ? { syntax: { name: "nodes", gate: "gate", columns: ["__path", "__start", "__end", "id", "kind"] } } : {}),
      query: syntax
        ? "SELECT __path path,__start start_byte,__end end_byte FROM nodes WHERE kind='module'"
        : "SELECT __path path,__start start_byte,__end end_byte FROM gate WHERE length(source_text('other0.py',0,80)||source_text('other1.py',0,80))>0",
      key: ["path"],
    }, violation: { code: "admitted-source", message: "Admitted source", anchor: "witness.span" } }],
  };
  const capabilities = JSON.parse(capabilitiesJson());
  capabilities.limits.maxSourceBytes = 128;
  const compiled = native.CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(policy)), "json", JSON.stringify(capabilities));
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(compiled);
    for (let index = 0; index < 3; index++) {
      await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: index + 1,
        files: [{ path: `other${index}.py`, language: "python", source: `other = '${"x".repeat(70)}'\n` }], removed: [], quiet: true, deferGlobal: true }));
    }
    await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 4, files: [], removed: [], quiet: true }));
    const empty = JSON.parse(await workspace.resultPageAsync(0, RESULT_PAGE_BYTES));
    expect(empty.failures).toEqual([]);
    expect(empty.violations).toEqual([]);
    if (syntax) {
      // Syntax consumers demand one file, not all source covered by the rule.
      await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 5,
        files: [{ path: "selected.py", language: "python", source: "selected = 1\n" }], removed: [], quiet: true }));
      const admitted = JSON.parse(await workspace.resultPageAsync(0, RESULT_PAGE_BYTES));
      expect(admitted.failures).toEqual([]);
      expect(admitted.violations).toMatchObject([{ path: "selected.py" }]);
    } else {
      // maxSourceBytes is a per-file limit. Several admitted files can exceed it in total.
      await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 5,
        files: [{ path: "selected.py", language: "python", source: "selected = 1\n" }], removed: [], quiet: true }));
      const admitted = JSON.parse(await workspace.resultPageAsync(0, RESULT_PAGE_BYTES));
      expect(admitted.failures).toEqual([]);
      expect(admitted.violations).toMatchObject([{ path: "selected.py" }]);
      // A single oversized source must still fail instead of returning partial text.
      await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 6,
        files: [{ path: "other0.py", language: "python", source: `other = '${"x".repeat(150)}'\n` }], removed: [], quiet: true }));
      const oversized = JSON.parse(await workspace.resultPageAsync(0, RESULT_PAGE_BYTES));
      expect(oversized.failures).toMatchObject([{ code: "source-incomplete", message: "A relational input is incomplete: gate: other0.py" }]);
    }
  } finally { workspace.close(); }
});

it("completes the legacy query on the recorded large intent source", async () => {
  // Reuse the complete pinned source already captured by the sixth-batch suite.
  // The unchanged legacy reporter returns no findings on this source.
  const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-06-recording.json"), "utf8"));
  const path = "impl/common/l2070_server_plan/steps/intent_versioning_extended.py";
  const file = recording.events.flatMap((event: { files?: { path: string; source: string }[] }) => event.files ?? [])
    .find((file: { path: string; source: string }) => file.path === path && file.source.length > 700_000);
  expect(file).toBeDefined();
  const policy = codeRuleSchema.parse(JSON.parse(readFileSync("packs/data-layer/candidates/rules/data-layer-legacy-execution-surfaces.json", "utf8")));
  const native = loadNativeBinding();
  const rule = native.CodeRuleDslRuntime.compile(JSON.stringify(policy), "json", capabilitiesJson());
  const workspace = new native.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(rule);
    await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 1, files: [{ ...file, language: "python" }], removed: [], quiet: true }));
    const page = JSON.parse(await workspace.resultPageAsync(0, RESULT_PAGE_BYTES));
    expect(page.failures).toEqual([]);
    expect(page.violations).toEqual([]);
  } finally { workspace.close(); }
}, 30_000);
