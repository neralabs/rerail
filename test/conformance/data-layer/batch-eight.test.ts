import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

it.each(["readthrough_authority_architecture", "authentication_plan_purity", "legacy_callback_quarantine", "lazy_adapter_ownership", "plan_helpers_allowlist", "lazy_sql_surface", "pure_lazy_runtime_purity", "eager_resolve", "lazy_architecture"])("ports original %s positives and clean cases", async check => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-eight-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-recording.json"), "utf8"));
    recording.checks = [check];
    recording.events = recording.events.filter((event: { check: string }) => event.check === recording.checks[0]);
    const input = join(directory, "input.json"), output = join(directory, "result.json");
    writeFileSync(input, JSON.stringify(recording));
    const supplemental = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-probes.json"), "utf8"));
    supplemental.checks = [check];
    supplemental.events = supplemental.events.filter((event: { check: string }) => event.check === check);
    const probes = join(directory, "probes.json"); writeFileSync(probes, JSON.stringify(supplemental));
    const passed = await replay([input, probes], output);
    const report = JSON.parse(readFileSync(output, "utf8"));
    expect(report.rows.filter((row: { falsePositives: unknown[]; falseNegatives: unknown[]; engineFailures: unknown[] }) => row.falsePositives.length || row.falseNegatives.length || row.engineFailures.length)).toEqual([]);
    expect(passed).toBe(true);
    expect(report.positiveCalls).toBeGreaterThan(0);
    expect(report.cleanCalls).toBeGreaterThan(0);
    expect(report).toMatchObject({
      hostGuardCalls: 0, falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingNativeCoverage: [] });
  } finally { log.mockRestore(); rmSync(directory, { recursive: true, force: true }); }
}, 30_000);

it("keeps positive oracle probes for all eighteen lazy architecture diagnostics", () => {
  const probes = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-probes.json"), "utf8"));
  const codes = new Set(probes.events.filter((event: { check: string }) => event.check === "lazy_architecture")
    .flatMap((event: { findings: { code: string }[] }) => event.findings.map(finding => finding.code)));
  for (let number = 1; number <= 18; number++) expect(codes.has(`LA-${String(number).padStart(3, "0")}`)).toBe(true);
});
