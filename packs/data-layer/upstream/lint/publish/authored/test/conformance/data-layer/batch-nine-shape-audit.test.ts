import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { NineReplay, type NineOracle } from "../../../conformance/data-layer/batch-nine.js";
import { runDataLayerAudit } from "../../../src/cli/index.js";
const oracle = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-09-shape-audit.json", "utf8")) as NineOracle;
for (const mode of ["snapshot", "workspace"] as const) it.each([...new Set(oracle.records.map(record => record.check))])(`${mode} preserves original source-shape results for %s`, async check => {
  const replay = new NineReplay();
  try {
    for (const record of oracle.records.filter(record => record.check === check))
      expect(await replay.run(oracle, record, mode), record.test).toEqual(record.expected);
  } finally { replay.close(); }
}, 30_000);
for (const mode of ["snapshot", "workspace"] as const) it.each(oracle.records.filter(record => record.check === "connection_structural_boundaries" || record.check === "bootstrap_engine_invariants"))(`${mode} preserves shipped behavior for $test`, async record => {
  const result = await runDataLayerAudit({ repositoryRoot: ".", packDirectory: "packs/data-layer", mode,
    ruleIds: [`data-layer-${record.check.replaceAll("_", "-")}`], snapshot: { format: "rerail.repository-snapshot/v1", revision: 0,
      files: record.files.map(file => ({ path: file.path, source: oracle.sources[file.sourceSha256]!, language: "python" })) } });
  expect(result.failures).toEqual([]);
  expect(result.violations.map(row => ({ code: row.code, path: row.path, line: row.span!.startLine + 1, message: row.message }))).toEqual(record.expected);
});
