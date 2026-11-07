import { policyPath } from "../../../conformance/data-layer/policies.js";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createHash } from "node:crypto";
import { expect, it, vi } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { fixturesPath } from "../../support/oracles.js";

const checks = ["readthrough_authority_architecture", "authentication_plan_purity", "legacy_callback_quarantine", "lazy_adapter_ownership", "plan_helpers_allowlist", "lazy_sql_surface", "pure_lazy_runtime_purity", "eager_resolve", "lazy_architecture", "cached_pure_lazy_allowlist"];

it("accounts for every upstream case and permits only the recorded invalid-encoding I/O boundary", () => {
  const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-recording.json"), "utf8"));
  const coverage = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-case-coverage.json"), "utf8"));
  expect(recording.collected_tests).toBe(456);
  expect(coverage.cases).toHaveLength(recording.collected_tests);
  expect(coverage.cases.map((row: { test: string }) => row.test).sort()).toEqual(Object.keys(recording.test_outcomes).sort());
  const retained = recording.events.filter((event: { retained_reason?: string }) => event.retained_reason !== undefined);
  expect(retained).toHaveLength(1);
  expect(retained[0].test).toBe("scripts/tests/test_lint_plan_helpers_allowlist.py::test_scan_file_tolerates_non_utf8");
  expect(retained[0].retained_reason).toContain("UnicodeDecodeError");
  const eligible = recording.events.filter((event: { retained_reason?: string; upstream_passed: boolean }) => !event.retained_reason && event.upstream_passed);
  expect(eligible).toHaveLength(320);
  expect(eligible.filter((event: { findings: unknown[] }) => event.findings.length)).toHaveLength(201);
  expect(coverage.cases.reduce((sum: number, row: { nativeScannerCalls: number }) => sum + row.nativeScannerCalls, 0)).toBe(320);
  for (const [path, sha256] of Object.entries(coverage.inputs)) {
    expect(createHash("sha256").update(readFileSync(path)).digest("hex")).toBe(sha256);
  }
});

it.each(checks)("ports original %s positives and clean cases", async check => {
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


it.each(checks)("rejects an always-clean %s detector on a recorded oracle positive", async check => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-eight-positive-control-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const original = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-recording.json"), "utf8"));
    const probes = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-probes.json"), "utf8"));
    const positive = [...original.events, ...probes.events].find((event: { check: string; upstream_passed: boolean; retained_reason?: string; findings?: unknown[] }) =>
      event.check === check && event.upstream_passed && event.retained_reason === undefined && (event.findings?.length ?? 0) > 0);
    expect(positive, `${check} needs an unchanged-oracle positive`).toBeDefined();
    original.checks = [check]; original.events = [positive];
    const input = join(directory, "input.json"), output = join(directory, "result.json");
    writeFileSync(input, JSON.stringify(original));
    const filename = `data-layer-${check.replaceAll("_", "-")}.json`;
    const policy = JSON.parse(readFileSync(policyPath("packs/data-layer/candidates/rules", filename.replace(/\.json$/u, "")), "utf8"));
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

it("preserves diagnostic text, ordered graph targets and first matching owner calls", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-batch-eight-exact-"));
  const log = vi.spyOn(console, "log").mockImplementation(() => {});
  try {
    const output = join(directory, "result.json");
    const passed = await replay([
      fixturesPath("host", "data-layer-port", "batch-08-exact-probes.json"),
      fixturesPath("host", "data-layer-port", "batch-08-edge-probes.json"),
    ], output);
    const report = JSON.parse(readFileSync(output, "utf8"));
    expect(report.rows.filter((row: { falsePositives: unknown[]; falseNegatives: unknown[]; engineFailures: unknown[] }) =>
      row.falsePositives.length || row.falseNegatives.length || row.engineFailures.length)).toEqual([]);
    expect(passed).toBe(true);
  } finally { log.mockRestore(); rmSync(directory, { recursive: true, force: true }); }
}, 30_000);

it("binds the completed native assertion suites to the current rules and adapters", () => {
  const receipt = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-native-assertions.json"), "utf8"));
  for (const path of ["snapshot", "workspace"]) {
    const run = receipt.executionPaths[path];
    expect(run).toMatchObject({ collected: 456, passed: 456, failed: 0, skipped: 0, semanticFallbacks: [] });
    expect(Object.keys(run.cases)).toHaveLength(456);
    expect(run.casesWithNativeCalls).toBe(Object.values(run.cases).filter((row: unknown) => {
      const value = row as { nativeDiagnosticCalls: number; nativeHelperCalls: number };
      return value.nativeDiagnosticCalls + value.nativeHelperCalls > 0;
    }).length);
    expect(run.nativeCalls).toBe(Object.values(run.cases).reduce((sum: number, row: unknown) => {
      const value = row as { nativeDiagnosticCalls: number; nativeHelperCalls: number };
      return sum + value.nativeDiagnosticCalls + value.nativeHelperCalls;
    }, 0));
    expect(run.instrumentationTranslations).toHaveLength(4);
    expect(run.cases["scripts/tests/test_lint_lazy_architecture.py::test_class_method_alias_for_self_call_resolution"].nativeHelperCalls).toBeGreaterThan(0);
  }
  for (const [check, digest] of Object.entries(receipt.policySha256))
    expect(createHash("sha256").update(readFileSync(policyPath("packs/data-layer/candidates/rules", `data-layer-${check.replaceAll("_", "-")}`))).digest("hex")).toBe(digest);
  for (const [path, digest] of Object.entries(receipt.implementationSha256))
    expect(createHash("sha256").update(readFileSync(path)).digest("hex")).toBe(digest);
  for (const [path, digest] of Object.entries(receipt.factPolicySha256))
    expect(createHash("sha256").update(readFileSync(`packs/data-layer/candidates/facts/${path}`)).digest("hex")).toBe(digest);
  for (const comparison of Object.values(receipt.completeComparisons))
    expect(comparison).toMatchObject({ apiObservations: 305, analyzerInputs: 85, differences: 0, semanticFallbacks: [] });
  for (const [path, digest] of Object.entries(receipt.oracleSha256))
    expect(createHash("sha256").update(readFileSync(fixturesPath("host", "data-layer-port", path))).digest("hex")).toBe(digest);
  expect(Object.keys(receipt.positiveControls.rejectedByCheck)).toHaveLength(9);
  expect(receipt.positiveControls.failedCases).toBeGreaterThan(100);
});

it("uses one production graph contract for source and supplied facts without an oracle mode", () => {
  for (const check of checks) {
    const policy = JSON.parse(readFileSync(policyPath("packs/data-layer/candidates/rules", `data-layer-${check.replaceAll("_", "-")}`), "utf8"));
    for (const clause of policy.clauses) expect(clause.define?.options ?? {}).not.toHaveProperty("oracleAnalyzer");
  }
  const source = JSON.parse(readFileSync("packs/data-layer/rules/data-layer-lazy-architecture.json", "utf8"));
  const supplied = JSON.parse(readFileSync("packs/data-layer/candidates/facts/lazy-analyzed.json", "utf8"));
  type Stage = { name: string; reachability?: { selection?: string } };
  const programs = [source, supplied].map(policy => policy.clauses.flatMap((clause: { relational?: { stages?: Stage[] } }) => clause.relational?.stages ?? []) as Stage[]);
  const select = (stages: Stage[]) => stages.filter(stage => stage.reachability?.selection === "memoized-first").sort((a,b) => a.name.localeCompare(b.name));
  expect(select(programs[0]!)).toHaveLength(5);
  expect(select(programs[1]!)).toEqual(select(programs[0]!));
});
