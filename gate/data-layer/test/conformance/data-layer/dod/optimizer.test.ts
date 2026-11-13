import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { runDataLayerAudit } from "../../../../src/cli/index.js";
const audit = JSON.parse(readFileSync("fixtures/host/data-layer-port/batch-10-dod-audit-20261106.json", "utf8"));
type Case = { id: string; rule: string; files: { path: string; source: string; language: string }[]; expected: unknown[] };
for (const record of (audit.cases as Case[]).filter(row => row.id.startsWith("optimizer-")))
  for (const mode of ["snapshot", "workspace"] as const) it(`preserves configured ${record.id} through ${mode}`, async () => {
    const result = await runDataLayerAudit({ repositoryRoot: ".", packDirectory: "packs/data-layer", ruleIds: [record.rule], mode,
      snapshot: { format: "rerail.repository-snapshot/v1", revision: 0, files: record.files } });
    expect(result.failures).toEqual([]);
    expect(result.violations.map(row => ({ code: row.code, path: row.path, line: row.span!.startLine + 1, message: row.message }))).toEqual(record.expected);
  });
