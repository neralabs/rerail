import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

it("rejects a correct eager code on the wrong qualified symbol", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-wrong-symbol-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-recording.json"), "utf8"));
    const positive = recording.events.find((event: { check: string; findings?: unknown[] }) => event.check === "eager_resolve" && (event.findings?.length ?? 0) > 0);
    const clean = recording.events.find((event: { check: string; retained_reason?: string; findings?: unknown[] }) => event.check === "eager_resolve" && !event.retained_reason && event.findings?.length === 0);
    positive.compare_fields = ["symbol"];
    for (const finding of positive.findings) finding.metadata = { symbol: "wrong.module.owner" };
    recording.events = [positive, clean]; recording.checks = ["eager_resolve"];
    const input = join(directory, "input.json"), output = join(directory, "output.json");
    writeFileSync(input, JSON.stringify(recording));
    expect(await replay(input, output)).toBe(false);
    const report = JSON.parse(readFileSync(output, "utf8"));
    expect(report.falseNegatives).toBe(positive.findings.length);
    expect(report.falsePositives).toBe(positive.findings.length);
    expect(report.engineFailures).toBe(0);
  } finally { log.mockRestore(); rmSync(directory, { recursive: true, force: true }); }
}, 30_000);
