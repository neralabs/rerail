import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { replay } from "../../../../conformance/data-layer/replay.js";
it("preserves constructor positives and clean results beside unrelated lexical inventory", async () => {
  const root = await mkdtemp(join(tmpdir(), "rerail-identity-inventory-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const output = join(root, "report.json");
    const passed = await replay("fixtures/host/data-layer-port/batch-10-identity-inventory.json", output);
    const report = JSON.parse(await readFile(output, "utf8"));
    const failures = report.rows.flatMap((row: { engineFailures: unknown[]; statelessFailures: unknown[] }) => [...row.engineFailures, ...row.statelessFailures]);
    expect(failures, JSON.stringify(failures)).toEqual([]);
    expect(report).toMatchObject({ falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0 });
    expect(report.positiveCalls).toBeGreaterThan(0); expect(report.cleanCalls).toBeGreaterThan(0);
    expect(passed).toBe(true);
  } finally { log.mockRestore(); await rm(root, { recursive: true, force: true }); }
}, 90_000);
