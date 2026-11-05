import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { runDataLayerAudit, runDataLayerCommand } from "../../../src/cli/index.js";

// Original pinned _collect_file_violations reports BEI035 at line 3 for each source.
// Compare public spans here. Reading variables.row.line concealed the defect.
for (const mode of ["snapshot", "workspace"] as const) {
  it.each(["pass\n\n# COLD_APPLICATION\n", "\n\n# COLD_APPLICATION\npass\n", "text = '''first\né COLD_APPLICATION\n'''\n"])(`${mode} reports the original raw-source location`, async source => {
    const result = await runDataLayerAudit({ repositoryRoot: ".", packDirectory: "packs/data-layer", mode,
      ruleIds: ["data-layer-bootstrap-engine-invariants"], snapshot: { format: "rerail.repository-snapshot/v1", revision: 0,
        files: [{ path: "l3200_metadata/bootstrap_state.py", source, language: "python" }] } });
    const line = source.startsWith("text") ? 2 : 3;
    expect(result.failures).toEqual([]);
    expect(result.violations.map(row => ({ code: row.code, line: row.span!.startLine + 1, message: row.message }))).toEqual([
      { code: "BEI035", line, message: "deleted cold-bootstrap surface 'COLD_APPLICATION' must not be reintroduced" },
    ]);
  });
  it(`${mode} preserves cache-health's configured plugin scope`, async () => {
    const result = await runDataLayerAudit({ repositoryRoot: ".", packDirectory: "packs/data-layer", mode,
      ruleIds: ["data-layer-bootstrap-cache-health"], snapshot: { format: "rerail.repository-snapshot/v1", revision: 0,
        files: [{ path: "conform/helpers.py", source: "def test_probe():\n    create_tables()\n", language: "python" }] } });
    expect(result.failures).toEqual([]);
    expect(result.violations.map(row => ({ code: row.code, line: row.span!.startLine + 1 }))).toEqual([{ code: "BCH003", line: 2 }]);
  });
}
for (const check of ["bootstrap-lazy-sql", "bootstrap-eager-resolve"]) it(`${check} retains report-only enforcement at the public command`, async () => {
  const document = JSON.parse(readFileSync(`packs/data-layer/rules/data-layer-${check}.json`, "utf8"));
  expect(document.mode).toBe("log");
  const root = await mkdtemp(join(tmpdir(), "rerail-report-only-"));
  try {
    const snapshot = join(root, "snapshot.json");
    await writeFile(snapshot, JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0,
      files: [{ path: "l3200_metadata/bootstrap_state.py", source: "table_exists_many()\n", language: "python" }] }));
    let output = "";
    const exit = await runDataLayerCommand(["--repo", root, "--pack", resolve("packs/data-layer"), "--snapshot", snapshot, "--rules", `data-layer-${check}`],
      { write(chunk: string) { output += chunk; return true; } });
    expect(JSON.parse(output).violations).toMatchObject([{ code: "BLB-004", mode: "log" }]);
    expect(exit).toBe(0);
  } finally { await rm(root, { recursive: true, force: true }); }
});
