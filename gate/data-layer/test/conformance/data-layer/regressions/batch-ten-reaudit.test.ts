import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { prepareDataLayerAudit, runDataLayerAudit } from "../../../../src/cli/index.js";
import { loadNativeBinding, RESULT_PAGE_BYTES } from "../../../../src/engine/index.js";
import { TenReplay, canonical, type TenOracle, type TenRecord } from "../../../../conformance/data-layer/batch-ten.js";

const oracle = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-reaudit.json", "utf8")) as TenOracle;
const records = oracle.records as (TenRecord & { production: boolean })[];
const checks = [...new Set(records.map(record => record.check))].sort();
it("retains original configured positives and clean controls for all ten audited rules", () => {
  expect(checks).toHaveLength(10);
  expect(records).toHaveLength(324);
  expect(new Set(records.map(record => record.test)).size).toBe(records.length);
  for (const check of checks) {
    const positive = records.find(record => record.test === check + "-control-positive");
    const clean = records.find(record => record.test === check + "-control-clean");
    expect(Array.isArray(positive?.expected) ? positive.expected.length : 0, check).toBeGreaterThan(0);
    expect(clean?.expected, check).toEqual([]);
  }
});
for (const record of records) for (const mode of ["snapshot", "workspace"] as const)
  it(`${mode} preserves original production boundaries: ${record.test}`, async () => {
    if (record.production) {
      const result = await runDataLayerAudit({ repositoryRoot: ".", packDirectory: "packs/data-layer", mode,
        ruleIds: ["data-layer-" + record.check.replaceAll("_", "-")],
        snapshot: { format: "rerail.repository-snapshot/v1", revision: 0,
          files: record.files.map(file => ({ path: file.path, source: file.source!, language: "python" })) } });
      expect(result.failures).toEqual([]);
      expect(canonical(result.violations.map(row => ({ path: row.path, line: row.span!.startLine + 1,
        code: row.code, message: row.message })))).toEqual(canonical(record.expected));
    } else {
      const replay = new TenReplay();
      try { expect(await replay.run(oracle, record, mode)).toEqual(record.expected); }
      finally { replay.close(); }
    }
  });
it("preserves HNSW file-to-directory scope changes in one persistent workspace", async () => {
  const outside = records.find(record => record.test === "hnsw_architecture:root-file")!;
  const inside = records.find(record => record.test === "hnsw_architecture:root-directory")!;
  const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
  try {
    const { compiled } = await prepareDataLayerAudit({ repositoryRoot: ".", packDirectory: "packs/data-layer",
      ruleIds: ["data-layer-hnsw-architecture"], snapshot: { format: "rerail.repository-snapshot/v1", revision: 0, files: [] } });
    for (const rule of compiled) workspace.addCodeRule(rule);
    let previous: string[] = [];
    for (const [index, record] of [outside, inside, outside, inside].entries()) {
      const paths = record.files.map(file => file.path);
      await workspace.applyAsync(JSON.stringify({ revision: index + 1, quiet: true,
        files: record.files.map(file => ({ path: file.path, source: file.source!, language: "python" })),
        removed: previous.filter(path => !paths.includes(path)) }));
      const actual = [];
      for (let cursor = 0; ;) {
        const page = JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES));
        expect(page.failures).toEqual([]);
        actual.push(...page.violations.map((row: { path: string; code: string; message: string; span: { startLine: number } }) =>
          ({ path: row.path, line: row.span.startLine + 1, code: row.code, message: row.message })));
        if (page.done) break;
        if (page.cursor <= cursor) throw new Error("Result cursor did not advance");
        cursor = page.cursor;
      }
      expect(canonical(actual), `revision ${index + 1}`).toEqual(canonical(record.expected));
      previous = paths;
    }
  } finally { workspace.close(); }
});
for (const check of checks)
  it(`rejects a disabled configured detector in the fresh audit: ${check}`, async () => {
    const record = records.find(record => record.test === check + "-control-positive")!;
    const replay = new TenReplay("packs/data-layer/rules", true);
    try {
      for (const mode of ["snapshot", "workspace"] as const)
        expect(await replay.run(oracle, record, mode, true)).not.toEqual(canonical(record.expected));
    } finally { replay.close(); }
  });
