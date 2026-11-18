import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { replay } from "../../../../conformance/data-layer/replay.js";

it.each(["lazy_architecture", "pure_lazy_runtime_purity"])("preserves original %s results with dense global inventory", async check => {
  const root = await mkdtemp(join(tmpdir(), "rerail-global-queries-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const fixture = JSON.parse(await readFile("fixtures/host/data-layer-port/batch-10-global-queries.json", "utf8"));
    fixture.checks = [check]; fixture.events = fixture.events.filter((event: { check: string }) => event.check === check);
    const input = join(root, "input.json"), output = join(root, "result.json");
    await writeFile(input, JSON.stringify(fixture));
    const passed = await replay(input, output);
    const result = JSON.parse(await readFile(output, "utf8"));
    const failures = result.rows.flatMap((row: { engineFailures: unknown[]; statelessFailures: unknown[] }) => [...row.engineFailures, ...row.statelessFailures]);
    expect(failures, JSON.stringify(failures)).toEqual([]);
    expect(result).toMatchObject({ falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0 });
    expect(result.positiveCalls).toBeGreaterThan(0); expect(result.cleanCalls).toBeGreaterThan(0);
    expect(passed).toBe(true);
  } finally { log.mockRestore(); await rm(root, { recursive: true, force: true }); }
}, 90_000);
