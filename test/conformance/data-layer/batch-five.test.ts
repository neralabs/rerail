import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

const candidates = new Set(["missing_async_pairs", "sync_async_signatures", "async_sync_delegation",
  "data_layer_facade_purity", "data_layer_compat_aliases", "layer_violations", "backend_coupling", "imperative_private_functions"]);

it("matches the recorded source corpus for the eight fifth-batch candidates without certifying repository parity", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-five-"));
  const paths = ["batch-05-recording.json", "batch-05-probes.json", "batch-05-graph-probes.json"].map(name => {
    const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", name), "utf8"));
    recording.checks = recording.checks.filter((check: string) => candidates.has(check));
    recording.events = recording.events.filter((event: { check: string }) => candidates.has(event.check));
    const path = join(directory, name);
    writeFileSync(path, JSON.stringify(recording));
    return path;
  });
  const output = join(directory, "result.json");
  expect(await replay(paths, output)).toBe(true);
  expect(JSON.parse(readFileSync(output, "utf8"))).toMatchObject({
    nativeCalls: 180, positiveCalls: 94, cleanCalls: 86,
    falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingCoverage: [],
  });
}, 60_000);
