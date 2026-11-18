import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";
import { expect, it } from "vitest";
import { NineReplay, type NineOracle } from "../../../conformance/data-layer/batch-nine.js";
import { readBundledCatalog, codeRuleSchema, loadNativeBinding, NativeCodeRuleEnforcer, type CodeRule } from "../../../src/engine/index.js";
import { capabilitiesJson } from "../../../conformance/engine.js";

const oracle = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-09-complete-api.json", "utf8")) as NineOracle;
const checks = ["connection_mode_split", "connection_structural_boundaries", "connection_service_architecture", "schema_architecture", "schema_bootstrap_contracts", "bootstrap_engine_invariants", "bootstrap_lazy_sql", "bootstrap_eager_resolve", "bootstrap_cache_health", "plan_composition_architecture"];
const unique = [...new Map(oracle.records.map(record => [JSON.stringify({ ...record, test: undefined }), record])).values()];

it("retains all upstream cases, complete results and independently observed sources", () => {
  expect(Object.keys(oracle.cases)).toHaveLength(228);
  for (const [digest, source] of Object.entries(oracle.sources)) expect(createHash("sha256").update(source).digest("hex")).toBe(digest);
  for (const check of checks) {
    const records = oracle.records.filter(r => r.check === check);
    expect(records.some(r => Array.isArray(r.expected) && r.expected.length > 0)).toBe(true);
    expect(records.some(r => Array.isArray(r.expected) && r.expected.length === 0)).toBe(true);
  }
  expect(oracle.records.some(r => r.test.includes("test_connection_service_architecture_linter_is_clean") && r.files.length === 100)).toBe(true);
  expect(oracle.records.some(r => r.test.includes("test_schema_architecture_fixture_names_do_not_control_findings"))).toBe(true);
  expect(Object.keys(oracle.cases).filter(name => name.startsWith("tests/l9875_mcp/test_mcp_architecture_contracts.py::"))).toHaveLength(4);
});

it.each(checks)("matches complete %s results through both native APIs", async check => {
  const replay = new NineReplay();
  try {
    for (const record of unique.filter(r => r.check === check)) for (const mode of ["snapshot", "workspace"] as const)
      expect(await replay.run(oracle, record, mode), `${mode}: ${record.test}: ${record.entry}`).toEqual(record.expected);
  } finally { replay.close(); }
}, 90_000);

it.each(checks)("rejects a disabled %s detector on a recorded positive", async check => {
  const record = oracle.records.find(r => r.check === check && Array.isArray(r.expected) && r.expected.length > 0)!;
  const replay = new NineReplay("packs/data-layer/rules", true);
  try { expect(await replay.run(oracle, record, "snapshot")).not.toEqual(record.expected); }
  finally { replay.close(); }
});

it("ships prior documents once within the expanded pack, with explicit ownership of the live probe contract", () => {
  const pack = readBundledCatalog(["packs/data-layer"]);
  expect(pack.documents.size).toBe(30);
  for (const check of checks) expect(pack.documents.has(`data-layer-${check.replaceAll("_", "-")}`)).toBe(true);
  const authored = JSON.parse(readFileSync("packs/data-layer/rules/data-layer-plan-composition-architecture.json", "utf8"));
  authored.scope = { include: ["**/*"], exclude: [] };
  const rule = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(codeRuleSchema.parse(authored)), "json", capabilitiesJson());
  const result = JSON.parse(rule.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, files: [], facts: [] }), "report"));
  expect(result.failures).toEqual([expect.objectContaining({ message: expect.stringContaining("composition observations required") })]);
});

it.each([
  { check: "connection_mode_split", entry: "_violations_for_files", path: "api/probe.py", excluded: "api/tests/probe.py" },
  { check: "bootstrap_cache_health", entry: "scan_file", path: "conform/test_probe.py", excluded: "conform/__pycache__/test_probe.py" },
  { check: "schema_architecture", entry: "scan_source", path: "l2000_connection/probe.py", excluded: "l2000_connection/tests/probe.py" },
])("reports and retracts $check findings with the shipped scope and default entry", async spec => {
  const record = oracle.records.find(r => r.check === spec.check && r.entry === spec.entry && Array.isArray(r.expected) && r.expected.length > 0 && r.files.length === 1)!;
  const source = oracle.sources[record.files[0]!.sourceSha256]!;
  const catalog = readBundledCatalog(["packs/data-layer"]);
  const rule = catalog.documents.get(`data-layer-${spec.check.replaceAll("_", "-")}`)!.rule as CodeRule;
  const root = await mkdtemp(join(tmpdir(), "rerail-batch-nine-production-"));
  const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog([rule]), { workspaceRoot: root, steerDelivery: { requestSteer: async () => undefined } });
  async function edit(revision: number, path: string, generatedText: string) {
    await enforcer.auditGeneratedEdits("batch-nine-production", { revision, checkpoints: ["edit"], edits: [{ path, language: "python", generatedText }] });
    await expect.poll(() => enforcer.snapshotStatus().pending, { timeout: 15_000 }).toBe(false);
    const result = enforcer.snapshotStatus(); expect(result.failures).toEqual([]); return result;
  }
  try {
    await enforcer.openWorkspace("batch-nine-production");
    const positive = await edit(1, spec.path, source);
    const expected = record.expected as { code?: string; message: string; line: number }[];
    expect(positive.violations.map(r => ({ message: r.message, line: r.span!.startLine + 1 })).sort((a,b) => a.message.localeCompare(b.message)))
      .toEqual(expected.map(r => ({ message: r.message, line: r.line })).sort((a,b) => a.message.localeCompare(b.message)));
    expect((await edit(2, spec.path, "pass\n")).violations).toEqual([]);
    expect((await edit(3, spec.excluded, source)).violations).toEqual([]);
    expect((await edit(4, "unrelated/probe.py", source)).violations).toEqual([]);
    expect((await edit(5, spec.path, source)).violations).toHaveLength(expected.length);
  } finally { await enforcer.close(); await rm(root, { recursive: true, force: true }); }
}, 30_000);

it("binds the final assertions, disabled controls and kernel evidence to their implementations", () => {
  const path = "fixtures/host/data-layer-port/batch-09-native-assertions.json";
  const receipt = JSON.parse(readFileSync(path, "utf8"));
  expect(receipt.sourcePin).toBe("2b1504adc4291daaf080b2e932962b134da1a69a");
  for (const mode of ["snapshot", "workspace"]) {
    expect(receipt.executionPaths[mode]).toMatchObject({ cases: 228, completeResults: 269, positiveResults: 156, rejected: [] });
    expect(receipt.executionPaths[mode].engineSha256).toBe(receipt.engineSha256);
    expect(Object.keys(receipt.executionPaths[mode].caseOutcomes)).toHaveLength(228);
    expect(receipt.executionPaths[mode].instrumentationTranslations).toHaveLength(1);
    for (const phases of Object.values(receipt.executionPaths[mode].caseOutcomes) as { outcome: string }[][])
      expect(phases.map(p => p.outcome)).toEqual(["passed", "passed", "passed"]);
  }
  expect(receipt.positiveControls.cases).toBe(228);
  expect(receipt.positiveControls.rejected).toEqual([...checks].sort());
  expect(receipt.positiveControls.failedCases).toBe(161);
  for (const [path, digest] of Object.entries(receipt.fileSha256))
    expect(createHash("sha256").update(readFileSync(path)).digest("hex"), path).toBe(digest);
});
