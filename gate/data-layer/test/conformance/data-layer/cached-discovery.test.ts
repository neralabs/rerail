import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
import { replayCachedDiscovery } from "../../../conformance/data-layer/cached-discovery.js";
import { fixturesPath } from "../../support/oracles.js";

it("matches every recorded cached declaration field, including positive helper tests", async () => {
  const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-recording.json"), "utf8"));
  expect(recording.helper_events.length).toBeGreaterThan(0);
  const report = await replayCachedDiscovery(recording.helper_events);
  expect(report.failures).toEqual([]);
  expect(report.differences).toEqual([]);
  expect(report.positive).toBeGreaterThan(0);
  expect(report.clean).toBeGreaterThan(0);
  for (const name of ["test_cached_policy_preserves_normalized_identifier_findings", "test_cached_policy_enumerates_only_candidate_module_asts_once"]) {
    expect(report.tests.some(test => test.includes(name))).toBe(true);
  }
}, 60_000);

it("rejects missing declarations and changed fingerprints in the oracle contract", async () => {
  const recording = JSON.parse(readFileSync(fixturesPath("host", "data-layer-port", "batch-08-recording.json"), "utf8"));
  const positive = recording.helper_events.find((event: { functions?: unknown[] }) => (event.functions?.length ?? 0) > 0);
  expect(positive).toBeDefined();
  const removed = structuredClone(positive); removed.functions = [];
  const changed = structuredClone(positive); changed.functions[0].body_hash = "0".repeat(64);
  const report = await replayCachedDiscovery([removed, changed]);
  expect(report.failures).toEqual([]);
  expect(report.differences).toHaveLength(4);
}, 30_000);
