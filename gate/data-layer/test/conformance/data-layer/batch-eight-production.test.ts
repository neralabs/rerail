import { mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { readBundledCatalog, NativeCodeRuleEnforcer, type CodeRule } from "../../../src/engine/index.js";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";

it("loads the thirty opt-in documents through the public pack catalog with one document per rule id", async () => {
  const catalog = readBundledCatalog(["packs/data-layer"]);
  expect(catalog.packs.get("data-layer")!.rules).toHaveLength(30);
  expect(catalog.documents.size).toBe(30);
  const candidates = await readdir("packs/data-layer/candidates/rules");
  expect(candidates).toHaveLength(70);
  for (const id of catalog.documents.keys()) expect(candidates).not.toContain(`${id}.json`);
});

it.each([
  { check: "authentication-plan-purity", test: "test_rejects_transport_and_mutable_results_in_semantic_steps", path: "impl/remote/steps.py", excluded: "impl/remote/test_steps.py" },
  { check: "pure-lazy-runtime-purity", test: "test_flags_protected_runtime_attr_access_inside_pure_lazy", path: "l7200_versioned_transactions/builder.py", excluded: "l7200_versioned_transactions/tests/builder.py" },
])("reports and retracts $check diagnostics through the enforcer using shipped scope and defaults", async spec => {
  const catalog = readBundledCatalog(["packs/data-layer"]);
  const rule = catalog.documents.get(`data-layer-${spec.check}`)!.rule as CodeRule;
  const recording = JSON.parse(await readFile("fixtures/host/data-layer-port/batch-08-recording.json", "utf8"));
  const event = recording.events.find((row: { test: string }) => row.test.endsWith(`::${spec.test}`));
  const source: string = event.files[0].source;
  const root = await mkdtemp(join(tmpdir(), "rerail-batch-eight-production-"));
  const requestSteer = vi.fn(async () => undefined);
  const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog([rule]), { workspaceRoot: root, steerDelivery: { requestSteer } });
  async function edit(revision: number, path: string, generatedText: string) {
    await enforcer.auditGeneratedEdits("batch-eight-production", { revision, checkpoints: ["edit"], edits: [{ path, language: "python", generatedText }] });
    await expect.poll(() => enforcer.snapshotStatus().pending, { timeout: 15_000 }).toBe(false);
    const result = enforcer.snapshotStatus(); expect(result.failures).toEqual([]); return result;
  }
  try {
    await enforcer.openWorkspace("batch-eight-production");
    const positive = await edit(1, spec.path, source);
    expect(positive.violations.map(row => ({ code: row.code, message: row.message, path: row.path, line: row.span!.startLine + 1 })))
      .toEqual(event.findings.map((row: { code: string; message: string; line: number }) => ({ code: row.code, message: row.message, path: spec.path, line: row.line })));
    expect(requestSteer).toHaveBeenCalled();
    expect((await edit(2, spec.path, "pass\n")).violations).toEqual([]);
    expect((await edit(3, spec.excluded, source)).violations).toEqual([]);
    expect((await edit(4, "unrelated/steps.py", source)).violations).toEqual([]);
  } finally { await enforcer.close(); await rm(root, { recursive: true, force: true }); }
}, 30_000);
