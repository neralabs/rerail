import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

it("does not certify native positive coverage from a host signature guard", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-native-positive-coverage-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-04-probes.json"), "utf8"));
    recording.checks = ["sql_grammar_escape_hatches"];
    recording.events = recording.events.filter((event: { test: string; check: string }) => event.check === "sql_grammar_escape_hatches" &&
      ["probe:sql_grammar_escape_hatches:invalid-signature-16:plugin", "probe:sql_grammar_escape_hatches:safe:plugin"].includes(event.test));
    expect(recording.events).toHaveLength(2);
    const input = join(directory, "recording.json"), output = join(directory, "result.json");
    writeFileSync(input, JSON.stringify(recording));
    expect(await replay(input, output)).toBe(false);
    expect(JSON.parse(readFileSync(output, "utf8"))).toMatchObject({
      nativeCalls: 1, hostGuardCalls: 1, positiveCalls: 1, cleanCalls: 1,
      falsePositives: 0, falseNegatives: 0, engineFailures: 0,
      missingCoverage: [], missingNativeCoverage: [{ check: "sql_grammar_escape_hatches", positive: 0, clean: 1 }],
    });
  } finally { log.mockRestore(); rmSync(directory, { recursive: true, force: true }); }
});
