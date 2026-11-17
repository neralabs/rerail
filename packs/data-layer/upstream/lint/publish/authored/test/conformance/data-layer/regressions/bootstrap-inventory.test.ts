import { readFileSync } from "node:fs";
import { gunzipSync } from "node:zlib";
import { expect, it } from "vitest";
import { NineReplay, type NineOracle } from "../../../../conformance/data-layer/batch-nine.js";

const oracle = JSON.parse(gunzipSync(readFileSync("fixtures/host/data-layer-port/batch-09-bootstrap-inventory.json.gz")).toString("utf8")) as NineOracle;
for (const mode of ["snapshot", "workspace"] as const) it(`${mode} preserves bootstrap positives and clean results with repository-sized inventory`, async () => {
  const replay = new NineReplay();
  try {
    expect(oracle.records.some(record => Array.isArray(record.expected) && record.expected.length > 0)).toBe(true);
    expect(oracle.records.some(record => Array.isArray(record.expected) && record.expected.length === 0)).toBe(true);
    for (const record of oracle.records) expect(await replay.run(oracle, record, mode), record.test).toEqual(record.expected);
  } finally { replay.close(); }
}, 90_000);
