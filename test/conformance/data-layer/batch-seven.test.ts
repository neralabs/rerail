import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

// These are the local shapes currently executable on the integrated production
// API. This is deliberately not a declaration that batch seven is complete.
const localChecks = [
  "executor_publication_repairs", "publication_registry_writes", "unknown_contract_defaults",
  "declared_fact_interrogation", "history_serverplan_projection", "revision_ancestry_invariants", "grammar_mutation_invalidation",
];

it.each(localChecks)("preserves original and supplemental %s findings through snapshot and workspace APIs", async check => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-seven-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const inputs = ["recording", "probes"].map(kind => {
      const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", `batch-07-${kind}.json`), "utf8"));
      recording.checks = [check];
      recording.events = recording.events.filter((event: { check: string }) => event.check === check);
      const input = join(directory, `${kind}.json`);
      writeFileSync(input, JSON.stringify(recording));
      return input;
    });
    const output = join(directory, "result.json");
    const passed = await replay(inputs, output);
    const report = JSON.parse(readFileSync(output, "utf8"));
    expect(report.rows.filter((row: { falsePositives: unknown[]; falseNegatives: unknown[] }) => row.falsePositives.length || row.falseNegatives.length)).toEqual([]);
    expect(report).toMatchObject({
      falsePositives: 0, falseNegatives: 0, engineFailures: 0,
      executionPathDifferences: 0, missingCoverage: [],
    });
    expect(report.nativeCalls).toBeGreaterThan(1);
    expect(report.positiveCalls).toBeGreaterThan(0);
    expect(report.cleanCalls).toBeGreaterThan(0);
    expect(passed).toBe(true);
  } finally {
    log.mockRestore();
    rmSync(directory, { recursive: true, force: true });
  }
}, 30_000);
