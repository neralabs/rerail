import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, it, vi } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

// Source observations, including real positives, are independent of the clean
// production repository. Every check must exercise the native engine itself.
const localChecks = [
  "executor_publication_repairs", "publication_registry_writes", "unknown_contract_defaults",
  "declared_fact_interrogation", "history_serverplan_projection", "revision_ancestry_invariants", "grammar_mutation_invalidation",
  "versioned_merged_identity", "post_op_no_internal_dispatch",
  "post_op_side_effects",
];

it("keeps positive fixtures for the covered diagnostic families", () => {
  const recordings = ["recording", "probes"].map(kind => JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", `batch-07-${kind}.json`), "utf8")));
  expect([...localChecks].sort()).toEqual([...recordings[0].checks].sort());
  const codes: Record<string, string[]> = {
    post_op_side_effects: ["POSTOP001", "POSTOP002", "POSTOP200", "POSTOP201", "POSTOP202", "POSTOP203", "POSTOP204", "POSTOP205", "POSTOP206"],
    post_op_no_internal_dispatch: Array.from({ length: 9 }, (_, index) => `POSTOPDISPATCH00${index + 1}`),
    executor_publication_repairs: ["executor_publication_repair"],
    history_serverplan_projection: ["HSP-001", "HSP-002", "HSP-003", "HSP-004"],
    revision_ancestry_invariants: ["forbidden_descriptor", "removed_surface", "stored_revision_text_cast"],
    versioned_merged_identity: ["VMI001", "VMI002", "VMI003"],
    publication_registry_writes: ["REGISTRY_DIRECT_WRITE"],
    declared_fact_interrogation: ["CONTRACT_ATTR_PROBE", "OVERRIDE_IDENTITY_PROBE"],
    grammar_mutation_invalidation: ["MUTATION_WITHOUT_INVALIDATION"],
    unknown_contract_defaults: ["UNKNOWN_DEFAULTED", "UNKNOWN_AS_FALSY"],
  };
  for (const [check, required] of Object.entries(codes)) {
    const events = recordings.flatMap(recording => recording.events).filter((event: { check: string; retained_reason?: string; upstream_passed: boolean }) =>
      event.check === check && event.retained_reason === undefined && event.upstream_passed);
    const observed = new Set(events.flatMap((event: { findings: { code: string }[] }) => event.findings.map(finding => finding.code)));
    for (const code of required) expect(observed.has(code), `${check}: missing positive fixture for ${code}`).toBe(true);
  }
});

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
      executionPathDifferences: 0, missingCoverage: [], hostGuardCalls: 0,
    });
    expect(report.nativeCalls).toBeGreaterThan(1);
    expect(report.positiveCalls).toBeGreaterThan(0);
    expect(report.cleanCalls).toBeGreaterThan(0);
    const nativeCoverage = report.nativeCoverage.find((row: { check: string }) => row.check === check);
    expect(nativeCoverage.positive).toBeGreaterThan(0);
    expect(nativeCoverage.clean).toBeGreaterThan(0);
    expect(passed).toBe(true);
  } finally {
    log.mockRestore();
    rmSync(directory, { recursive: true, force: true });
  }
}, 30_000);

it.each(localChecks)("rejects an always-clean %s detector on an original upstream positive", async check => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-seven-positive-control-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-07-recording.json"), "utf8"));
    const positive = recording.events.find((event: { check: string; upstream_passed: boolean; retained_reason?: string; findings?: unknown[] }) =>
      event.check === check && event.upstream_passed && event.retained_reason === undefined && (event.findings?.length ?? 0) > 0);
    expect(positive, `${check} needs an original upstream positive`).toBeDefined();
    recording.checks = [check]; recording.events = [positive];
    const input = join(directory, "recording.json"), output = join(directory, "result.json");
    writeFileSync(input, JSON.stringify(recording));
    const filename = `data-layer-${check.replaceAll("_", "-")}.json`;
    const policy = JSON.parse(readFileSync(`packs/data-layer/candidates/rules/${filename}`, "utf8"));
    // Preserve the original option declarations consumed by the replay adapter.
    policy.clauses = [...policy.clauses.filter((clause: { define?: unknown }) => clause.define !== undefined), {
      id: "negative-control", forbid: { regex: { source: "file.content", pattern: "\\b\\B" } },
      violation: { code: "CONTROL", message: "Always clean control", anchor: "witness.span" },
    }];
    writeFileSync(join(directory, filename), JSON.stringify(policy));
    expect(await replay(input, output, directory)).toBe(false);
    const report = JSON.parse(readFileSync(output, "utf8"));
    expect(report.nativeCalls).toBe(1);
    expect(report.hostGuardCalls).toBe(0);
    expect(report.falseNegatives).toBe(positive.findings.length);
    expect(report.falsePositives).toBe(0);
    expect(report.engineFailures).toBe(0);
    expect(report.executionPathDifferences).toBe(0);
  } finally { log.mockRestore(); rmSync(directory, { recursive: true, force: true }); }
}, 30_000);

it("replays authority and reachable-symbol observations from the unchanged tests", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-seven-authority-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-07-recording.json"), "utf8"));
    recording.checks = ["post_op_side_effects"];
    recording.events = recording.events.filter((event: { entry: string }) =>
      ["_collect_broad_runtime_authority_violations", "_collect_exact_runtime_symbol_coverage_violations"].includes(event.entry));
    // Three direct unit observations and two helper calls inside a signing test.
    // The latter do not certify the surrounding signing workflow.
    expect(recording.events).toHaveLength(5);
    expect(recording.events.filter((event: { findings: unknown[] }) => event.findings.length > 0)).toHaveLength(2);
    const input = join(directory, "recording.json"), output = join(directory, "result.json");
    writeFileSync(input, JSON.stringify(recording));
    expect(await replay(input, output)).toBe(true);
    const report = JSON.parse(readFileSync(output, "utf8"));
    expect(report).toMatchObject({ nativeCalls: 5, hostGuardCalls: 0, positiveCalls: 2, cleanCalls: 3,
      falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0 });
  } finally { log.mockRestore(); rmSync(directory, { recursive: true, force: true }); }
});
