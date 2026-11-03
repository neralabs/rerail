import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

it("ports the original positive and clean executor-publication observations through snapshot and workspace APIs", async () => {
  const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-07-recording.json"), "utf8"));
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-seven-"));
  const input = join(directory, "recording.json"), output = join(directory, "result.json");
  recording.checks = ["executor_publication_repairs"];
  recording.events = recording.events.filter((event: { check: string }) => event.check === "executor_publication_repairs");
  writeFileSync(input, JSON.stringify(recording));
  expect(await replay(input, output)).toBe(true);
  expect(JSON.parse(readFileSync(output, "utf8"))).toMatchObject({
    nativeCalls: 2, positiveCalls: 1, cleanCalls: 1, falsePositives: 0, falseNegatives: 0,
    engineFailures: 0, executionPathDifferences: 0, missingCoverage: [],
  });
}, 30_000);
