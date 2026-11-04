import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

it.each(["readthrough_authority_architecture", "authentication_plan_purity", "legacy_callback_quarantine", "lazy_adapter_ownership"])("ports original %s positives and clean cases", async check => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-eight-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-recording.json"), "utf8"));
    recording.checks = [check];
    recording.events = recording.events.filter((event: { check: string }) => event.check === recording.checks[0]);
    const input = join(directory, "input.json"), output = join(directory, "result.json");
    writeFileSync(input, JSON.stringify(recording));
    expect(await replay(input, output)).toBe(true);
    const report = JSON.parse(readFileSync(output, "utf8"));
    expect(report.positiveCalls).toBeGreaterThan(0);
    expect(report.cleanCalls).toBeGreaterThan(0);
    expect(report).toMatchObject({
      hostGuardCalls: 0, falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingNativeCoverage: [] });
  } finally { log.mockRestore(); rmSync(directory, { recursive: true, force: true }); }
});
