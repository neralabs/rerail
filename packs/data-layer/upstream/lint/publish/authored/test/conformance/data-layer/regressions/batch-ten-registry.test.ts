import { readFileSync } from "node:fs";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { expect, it } from "vitest";
import { runDataLayerCommand } from "../../../../src/cli/index.js";
import { canonical, readTenOracle } from "../../../../conformance/data-layer/batch-ten.js";

const inventory = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-original-test-inventory.json", "utf8"));
const oracle = readTenOracle("fixtures/host/data-layer-port/batch-10-configured-defaults.json.gz");
for (const check of ["graph_lazy_sql_plan", "conditional_architecture", "embedding_vector_orchestration"])
  it(`preserves original registry CI blocking with the shipped ${check} rule`, async () => {
    const original = `scripts/tests/test_lint_all.py::test_serverplan_linters_are_ci_blockers[${check}]`;
    expect(inventory.suiteIntegrationCases[original]).toBeDefined();
    const id = "data-layer-" + check.replaceAll("_", "-");
    const policy = JSON.parse(readFileSync(`packs/data-layer/rules/${id}.json`, "utf8"));
    expect(policy.mode).toBe("block");
    const root = await mkdtemp(join(tmpdir(), "rerail-original-registry-"));
    try {
      for (const variant of ["positive", "clean"]) {
        const record = oracle.records.find(record => record.test === check + ":" + variant)!;
        const snapshot = join(root, "snapshot.json");
        await writeFile(snapshot, JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
          files: record.files.map(file => ({ path: file.path, source: file.source ?? oracle.sources[file.sourceSha256], language: "python" })) }));
        let output = "";
        const status = await runDataLayerCommand(["--repo", root, "--pack", resolve("packs/data-layer"),
          "--snapshot", snapshot, "--rules", id, "--json", "--check"], { write(chunk: string) { output += chunk; return true; } });
        expect(status, variant).toBe(variant === "positive" ? 1 : 0);
        const result = JSON.parse(output);
        expect(result.failures).toEqual([]);
        expect(result.violations.every((row: { mode: string }) => row.mode === "block")).toBe(true);
        expect(canonical(result.violations.map((row: { code: string; path: string; message: string; span: { startLine: number } }) =>
          ({ path: row.path, line: row.span.startLine + 1, code: row.code, message: row.message })))).toEqual(canonical(record.expected));
      }
    } finally { await rm(root, { recursive: true, force: true }); }
  });
