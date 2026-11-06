import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { runDataLayerAudit } from "../../../../src/cli/index.js";
import { TenReplay, canonical, type TenOracle, type TenRecord } from "../../../../conformance/data-layer/batch-ten.js";

const oracle = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-reaudit.json", "utf8")) as TenOracle;
const records = oracle.records as (TenRecord & { production: boolean })[];
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
