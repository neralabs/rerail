import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { readBundledCatalog, type CodeRule } from "../../../src/engine/index.js";
import { NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";

it("loads the ten opt-in documents through the public pack catalog with no candidate drift", async () => {
  const catalog = readBundledCatalog(["packs/data-layer"]);
  expect(catalog.packs.get("data-layer")!.rules).toHaveLength(10);
  expect(catalog.documents.size).toBe(10);
  for (const id of catalog.documents.keys()) {
    expect(await readFile(`packs/data-layer/rules/${id}.json`, "utf8"))
      .toBe(await readFile(`packs/data-layer/candidates/rules/${id}.json`, "utf8"));
  }
});

it("reports and retracts real batch-eight diagnostics through the enforcer using the shipped scope and defaults", async () => {
  const catalog = readBundledCatalog(["packs/data-layer"]);
  const rule = catalog.documents.get("data-layer-authentication-plan-purity")!.rule as CodeRule;
  const recording = JSON.parse(await readFile("fixtures/host/data-layer-port/batch-08-recording.json", "utf8"));
  const event = recording.events.find((row: { test: string }) => row.test.endsWith("::test_rejects_transport_and_mutable_results_in_semantic_steps"));
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
    const positive = await edit(1, "impl/remote/steps.py", source);
    expect(positive.violations.map(row => ({ code: row.code, message: row.message, path: row.path })))
      .toEqual(event.findings.map((row: { code: string; message: string }) => ({ code: row.code, message: row.message, path: "impl/remote/steps.py" })));
    expect(requestSteer).toHaveBeenCalled();
    expect((await edit(2, "impl/remote/steps.py", "class RemoteIntentPlanStep:\n    pass\n")).violations).toEqual([]);
    expect((await edit(3, "impl/remote/test_steps.py", source)).violations).toEqual([]);
    expect((await edit(4, "unrelated/steps.py", source)).violations).toEqual([]);
  } finally { await enforcer.close(); await rm(root, { recursive: true, force: true }); }
}, 30_000);
