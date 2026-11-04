import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

it("ports the original read-through authority positives and clean cases", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-eight-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-recording.json"), "utf8"));
    recording.checks = ["readthrough_authority_architecture"];
    recording.events = recording.events.filter((event: { check: string }) => event.check === recording.checks[0]);
    const input = join(directory, "input.json"), output = join(directory, "result.json");
    writeFileSync(input, JSON.stringify(recording));
    expect(await replay(input, output)).toBe(true);
    expect(JSON.parse(readFileSync(output, "utf8"))).toMatchObject({ nativeCalls: 9, positiveCalls: 7, cleanCalls: 2,
      hostGuardCalls: 0, falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingNativeCoverage: [] });
  } finally { log.mockRestore(); rmSync(directory, { recursive: true, force: true }); }
});
