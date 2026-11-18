import { mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { z } from "zod";
import { expect, it } from "vitest";
import { replay } from "../../../conformance/data-layer/replay.js";
import { loadNativeBinding } from "../../../src/engine/index.js";
import { capabilitiesJson } from "../../../conformance/engine.js";
import { fixturesPath } from "../../support/oracles.js";

const recording = fixturesPath("host", "data-layer-port", "batch-01-recording.json");
const secondRecording = fixturesPath("host", "data-layer-port", "batch-02-recording.json");
const secondProbes = fixturesPath("host", "data-layer-port", "batch-02-probes.json");
const thirdRecording = fixturesPath("host", "data-layer-port", "batch-03-recording.json");
const thirdProbes = fixturesPath("host", "data-layer-port", "batch-03-probes.json");
const policies = new URL("../../../packs/data-layer/candidates/rules/", import.meta.url);
const summarySchema = z.object({ replayedCalls: z.number(), positiveCalls: z.number(), cleanCalls: z.number(),
  falsePositives: z.number(), falseNegatives: z.number(), engineFailures: z.number(), executionPathDifferences: z.number(), missingCoverage: z.array(z.unknown()) });

async function run(policyDirectory?: string, recorded = recording) {
  const output = join(mkdtempSync(join(tmpdir(), "rerail-data-layer-gate-")), "report.json");
  const passed = await replay(recorded, output, policyDirectory);
  return { passed, summary: summarySchema.parse(JSON.parse(readFileSync(output, "utf8"))) };
}

function sentinel(pattern: string): string {
  const directory = mkdtempSync(join(tmpdir(), "rerail-data-layer-sentinel-"));
  const schema = z.object({ id: z.string(), clauses: z.array(z.object({ id: z.string(), violation: z.object({
    code: z.union([z.string(), z.object({ $option: z.string() }).passthrough()]),
  }).optional() })) });
  for (const file of readdirSync(policies)) {
    const original = schema.parse(JSON.parse(readFileSync(new URL(file, policies), "utf8")));
    const policy = { format: "rerail.policy/v1", id: original.id, revision: 1, mode: "block",
      scope: { include: ["**/*.py"], exclude: [] }, clauses: original.clauses.filter(clause => clause.violation !== undefined).map(clause => ({
        id: clause.id, forbid: { regex: { source: "file.content", pattern } },
        violation: { code: typeof clause.violation!.code === "string" ? clause.violation!.code : "SENTINEL", anchor: "witness.span", message: "Deliberately incorrect detector" },
      })) };
    writeFileSync(join(directory, file), JSON.stringify(policy));
  }
  return directory;
}

it("ports the first ten checks on the mechanically recorded positive and clean corpus", async () => {
  const { passed, summary } = await run();
  expect(passed).toBe(true);
  expect(summary).toEqual({ replayedCalls: 317, positiveCalls: 234, cleanCalls: 83,
    falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingCoverage: [] });
});

it("ports the second ten checks on the unchanged upstream positive and clean corpus", async () => {
  const { passed, summary } = await run(undefined, secondRecording);
  expect(passed).toBe(true);
  expect(summary).toEqual({ replayedCalls: 275, positiveCalls: 171, cleanCalls: 104,
    falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingCoverage: [] });
});

it("preserves source-order, alias, scope, and production-reporter behavior in second-batch oracle probes", async () => {
  const { passed, summary } = await run(undefined, secondProbes);
  expect(passed).toBe(true);
  expect(summary).toEqual({ replayedCalls: 116, positiveCalls: 74, cleanCalls: 42,
    falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingCoverage: [] });
});

it("ports the third ten checks from unchanged upstream tests and AST compatibility visitors", async () => {
  const { passed, summary } = await run(undefined, thirdRecording);
  expect(passed).toBe(true);
  expect(summary).toEqual({ replayedCalls: 194, positiveCalls: 116, cleanCalls: 78,
    falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingCoverage: [] });
}, 30_000);

it("preserves mutation, ownership, exception, category, and cache behavior in third-batch oracle probes", async () => {
  const { passed, summary } = await run(undefined, thirdProbes);
  expect(passed).toBe(true);
  expect(summary).toEqual({ replayedCalls: 167, positiveCalls: 76, cleanCalls: 91,
    falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingCoverage: [] });
}, 30_000);

it("rejects an empty detector for the third batch", async () => {
  const { passed, summary } = await run(sentinel("\\b\\B"), thirdRecording);
  expect(passed).toBe(false);
  expect(summary.falseNegatives).toBe(132);
  expect(summary.engineFailures).toBe(0);
}, 30_000);

it("rejects an always-reporting detector for the third batch", async () => {
  const { passed, summary } = await run(sentinel("\\A"), thirdRecording);
  expect(passed).toBe(false);
  expect(summary.falsePositives).toBeGreaterThan(0);
  expect(summary.engineFailures).toBe(0);
}, 30_000);

it("rejects swallowed-exception severity mismatches at otherwise identical locations", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-data-layer-severity-"));
  for (const file of readdirSync(policies)) {
    const text = readFileSync(new URL(file, policies), "utf8");
    writeFileSync(join(directory, file), file === "data-layer-swallowed-exceptions.json"
      ? text.replaceAll('"code": "CRITICAL"', '"code": "WARNING"') : text);
  }
  const { passed, summary } = await run(directory, thirdRecording);
  expect(passed).toBe(false);
  expect(summary.falsePositives).toBeGreaterThan(0);
  expect(summary.falseNegatives).toBeGreaterThan(0);
  expect(summary.engineFailures).toBe(0);
}, 30_000);

it("rejects an empty detector for the second batch", async () => {
  const { passed, summary } = await run(sentinel("\\b\\B"), secondRecording);
  expect(passed).toBe(false);
  expect(summary.falseNegatives).toBe(191);
  expect(summary.engineFailures).toBe(0);
});

it("rejects an always-reporting detector for the second batch", async () => {
  const { passed, summary } = await run(sentinel("\\A"), secondRecording);
  expect(passed).toBe(false);
  expect(summary.falsePositives).toBeGreaterThan(0);
  expect(summary.engineFailures).toBe(0);
});

it("rejects a wrong diagnostic category even when legacy code, line, and multiplicity agree", async () => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-data-layer-category-"));
  for (const file of readdirSync(policies)) {
    const text = readFileSync(new URL(file, policies), "utf8");
    writeFileSync(join(directory, file), file === "data-layer-type-safety-patterns.json" ? text.replace('"id": "text-str_union_expression"', '"id": "text-wrong-category"') : text);
  }
  const { passed, summary } = await run(directory, secondRecording);
  expect(passed).toBe(false);
  expect(summary.falsePositives).toBeGreaterThan(0);
  expect(summary.falseNegatives).toBeGreaterThan(0);
  expect(summary.engineFailures).toBe(0);
});

it("rejects an empty detector on the positive corpus", async () => {
  const { passed, summary } = await run(sentinel("\\b\\B"));
  expect(passed).toBe(false);
  expect(summary.falseNegatives).toBe(297);
  expect(summary.engineFailures).toBe(0);
});

it("rejects a detector that reports every file on the clean corpus", async () => {
  const { passed, summary } = await run(sentinel("\\A"));
  expect(passed).toBe(false);
  expect(summary.falsePositives).toBeGreaterThan(0);
  expect(summary.engineFailures).toBe(0);
});

it("distinguishes parameter declarations and catches deleted helper attributes", () => {
  const policy = readFileSync(new URL("data-layer-serverplan-grammar-surface.json", policies), "utf8");
  const compiled = loadNativeBinding().CodeRuleDslRuntime.compile(policy, "json", capabilitiesJson());
  const result = JSON.parse(compiled.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [],
    files: [{ path: "example.py", language: "python", source: "def build(SqlPrerenderedDdl: SqlPrerenderedDdl):\n    return conn.execute_grammar_direct()\n" }],
  }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((finding: { code: string }) => finding.code)).toEqual(["SPG-ddl_escape", "SPG-direct_helper_attribute"]);
});


it.each([
  ["no-source-inspection", "tests/.pytest_cache/example.py", "inspect.getsource(obj)\n", []],
  ["backend-type-strings", "impl/service.py", "backend  == 'postgres'\n", [["BT000", 1]]],
  ["removed-compat-surfaces", "impl/service.py", "value = TimerClaim\nvalue = obj.TimerClaim\n", [["RCS001", 1], ["RCS001", 2]]],
  ["versioned-lifecycle-legacy-surfaces", "impl/service.py", "class BackendLazyVersionedWriteTransactionSparse:\n    @decorator\n    async def commit_async(self):\n        pass\n", [["VLL001", 3]]],
])("covers the oracle-verified %s edge case", (name, path, source, codes) => {
  const policy = readFileSync(new URL(`data-layer-${name}.json`, policies), "utf8");
  const compiled = loadNativeBinding().CodeRuleDslRuntime.compile(policy, "json", capabilitiesJson());
  const result = JSON.parse(compiled.evaluate(JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [],
    files: [{ path, language: "python", source }],
  }), "report"));
  expect(result.failures).toEqual([]);
  expect(result.violations.map((finding: { code: string; span: { startLine: number } }) => [finding.code, finding.span.startLine + 1])).toEqual(codes);
});

it("matches fourth-batch upstream detections without claiming missing positive coverage", async () => {
  const { passed, summary } = await run(undefined, fixturesPath("host", "data-layer-port", "batch-04-recording.json"));
  expect(passed).toBe(false);
  expect(summary).toEqual({ replayedCalls: 206, positiveCalls: 130, cleanCalls: 76,
    falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0,
    missingCoverage: [{ check: "grammar_allowlist_burndown", positive: 0, clean: 0 },
      { check: "rust_sql_grammar_boundary", positive: 0, clean: 0 }] });
}, 60_000);

it("ports the fourth batch across the expanded positive and clean oracle corpus", async () => {
  const { passed, summary } = await run(undefined, fixturesPath("host", "data-layer-port", "batch-04-probes.json"));
  expect(passed).toBe(true);
  expect(summary).toEqual({ replayedCalls: 367, positiveCalls: 218, cleanCalls: 149,
    falsePositives: 0, falseNegatives: 0, engineFailures: 0, executionPathDifferences: 0, missingCoverage: [] });
}, 60_000);

it("certifies fourth-batch coverage from the original tests and supplemental corpus together", async () => {
  const output = join(mkdtempSync(join(tmpdir(), "rerail-data-layer-fourth-combined-")), "report.json");
  const passed = await replay([
    fixturesPath("host", "data-layer-port", "batch-04-recording.json"),
    fixturesPath("host", "data-layer-port", "batch-04-probes.json"),
  ], output);
  expect(passed).toBe(true);
  expect(summarySchema.parse(JSON.parse(readFileSync(output, "utf8")))).toEqual({
    replayedCalls: 573, positiveCalls: 348, cleanCalls: 225, falsePositives: 0, falseNegatives: 0,
    engineFailures: 0, executionPathDifferences: 0, missingCoverage: [],
  });
}, 60_000);

it("refuses to combine recordings from different source commits", async () => {
  const original = fixturesPath("host", "data-layer-port", "batch-04-recording.json");
  const directory = mkdtempSync(join(tmpdir(), "rerail-data-layer-source-pin-"));
  const modified = z.object({ commit: z.string() }).passthrough().parse(JSON.parse(readFileSync(original, "utf8")));
  modified.commit = "different-source";
  const other = join(directory, "recording.json");
  writeFileSync(other, JSON.stringify(modified));
  await expect(replay([original, other], join(directory, "report.json"))).rejects.toThrow("same source commit");
});

it("rejects an empty fourth-batch detector on the original positive corpus", async () => {
  const { passed, summary } = await run(sentinel("\\b\\B"), fixturesPath("host", "data-layer-port", "batch-04-recording.json"));
  expect(passed).toBe(false);
  expect(summary.falseNegatives).toBe(146);
  expect(summary.engineFailures).toBe(0);
}, 60_000);

it("rejects an always-reporting fourth-batch detector on the original clean corpus", async () => {
  const { passed, summary } = await run(sentinel("\\A"), fixturesPath("host", "data-layer-port", "batch-04-recording.json"));
  expect(passed).toBe(false);
  expect(summary.falsePositives).toBeGreaterThan(0);
  expect(summary.engineFailures).toBe(0);
}, 60_000);
