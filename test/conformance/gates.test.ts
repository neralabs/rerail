/**
 * The folded gate (conformance/languages/gates.ts): every rule of a gated tool-pack through loads, compact and the
 * conformance checks, judged on the pack's conformance report and gates.yaml's declared facts, and the summary and
 * sticky comment built from it.
 */
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { COMMENT_MARKER, renderComment } from "../../conformance/languages/comment.js";
import { loadEngine } from "../../conformance/languages/checks/engine.js";
import { judgeToolPack } from "../../conformance/languages/gates.js";
import { loadPack } from "../../conformance/languages/pack.js";
import { reportFailures, runPack } from "../../conformance/languages/runner.js";
import { summarize } from "../../conformance/languages/summary.js";
import { fixturesPath } from "../support/oracles.js";

const GATES = (recommended: string) => `adapter: bandit
rerail:
  pack: valid
  project: examplelint
upstream:
  name: example/examplelint
  repository: https://github.com/example/examplelint
  commit: ${"a".repeat(40)}
  version: 1.0.0
  license: MIT
  recommended_configs: [default]
  runtime: examplelint-1.0.0
  lints: [".py"]
popularity:
  observed_on: 2026-10-06
  evidence: [a fake upstream]
rules:
  EX101:
    deprecated: null
    recommended: ${recommended}
`;

const UNGATED = { checks: [], notGated: [], pending: [] };

let root: string;
let pack: string;

beforeEach(() => {
  // The `valid` pack as a gated tool-pack: examplelint's EX101 ported as rules/assert-used.json; rules/pass-statement.json
  // ships with no conformance.json entry.
  root = mkdtempSync(join(tmpdir(), "rerail-gates-"));
  pack = join(root, "packs", "valid");
  cpSync(fixturesPath("packs", "valid"), pack, { recursive: true });
  writeFileSync(join(pack, "upstream", "examplelint", "gates.yaml"), GATES("{ config: default, options: [] }"));
});

afterEach(() => rmSync(root, { recursive: true, force: true }));

const setUnits = (units: Record<string, { rules: string[] }>): void => {
  const path = join(pack, "upstream", "examplelint", "fixtures.expected.json");
  const recording = JSON.parse(readFileSync(path, "utf8")) as { source: Record<string, unknown> };
  writeFileSync(path, JSON.stringify({ ...recording, source: { ...recording.source, units } }));
};

async function judge() {
  const loaded = loadPack(pack);
  const report = await runPack(loaded, { fixtures: true, samples: false, date: "2026-01-01" });
  const verdict = judgeToolPack(root, loadEngine(), loaded, report, "examplelint");
  return { report, verdict, summary: summarize([{ pack: "valid", failures: reportFailures(report) }], [verdict], UNGATED) };
}

describe("a gated tool-pack's rules", () => {
  it("fail stays-silent without an own silent upstream unit or clean sample, and pass once the test data names one", async () => {
    const before = await judge();
    const rule = before.verdict.rules[0]!;
    expect(rule).toMatchObject({ id: "assert-used", location: "shipped", pass: false, stoppedAt: "conformance", failure: { check: "stays-silent" } });
    expect(before.summary.blocking).toContainEqual(expect.objectContaining({ pack: "valid/examplelint", rule: "assert-used", check: "stays-silent" }));
    // The report itself passes: stays-silent is the gate's alone.
    expect(reportFailures(before.report)).toEqual([]);

    setUnits({ "asserts.py": { rules: ["EX101"] }, "clean.py": { rules: ["EX101"] } });
    const after = await judge();
    expect(after.verdict.rules[0]).toMatchObject({ pass: true, stoppedAt: null, failure: null, defaultOn: true, on: true });
    expect(after.verdict.checks.map(check => [check.id, check.ok])).toEqual([["bundled-load", true], ["declared-pins", true], ["conformance-runs", true]]);
  });

  it("report a shipped document with no conformance.json entry as blocking, once", async () => {
    const { verdict, summary } = await judge();
    expect(verdict.unmapped).toEqual([{ file: "packs/valid/rules/pass-statement.json", location: "shipped" }]);
    expect(summary.blocking.filter(row => row.stoppedAt === "Mapping")).toEqual([
      { pack: "valid", rule: "packs/valid/rules/pass-statement.json", stoppedAt: "Mapping", check: "conformance-entry", why: "not mapped to an upstream rule: add its conformance.json entry" },
    ]);
  });

  it("take on or off from gates.yaml's declared facts, and list a shipped rule whose pack.json on disagrees", async () => {
    setUnits({ "clean.py": { rules: ["EX101"] } });
    writeFileSync(join(pack, "upstream", "examplelint", "gates.yaml"), GATES("null"));
    const { verdict, summary } = await judge();
    expect(verdict.rules[0]).toMatchObject({ pass: true, defaultOn: false, on: true });
    expect(summary.onOff).toEqual([expect.objectContaining({ pack: "valid/examplelint", rule: "assert-used", packJson: true, upstreamStatus: false })]);
  });

  it("fail every rule when the declared pins disagree with conformance.json", async () => {
    setUnits({ "clean.py": { rules: ["EX101"] } });
    writeFileSync(join(pack, "upstream", "examplelint", "gates.yaml"), GATES("{ config: default, options: [] }").replace("version: 1.0.0", "version: 1.0.1"));
    const { verdict } = await judge();
    expect(verdict.rules[0]).toMatchObject({ pass: false, stoppedAt: "tool-pack", failure: { check: "declared-pins" } });
  });

  it("are a candidate's report only: a failing candidate blocks nothing, a passing one is ready to promote", async () => {
    mkdirSync(join(pack, "candidates", "rules"), { recursive: true });
    cpSync(join(pack, "rules", "assert-used.json"), join(pack, "candidates", "rules", "assert-used.json"));
    rmSync(join(pack, "rules", "assert-used.json"));
    const conformance = JSON.parse(readFileSync(join(pack, "conformance.json"), "utf8")) as { rules: { policy: string }[] };
    writeFileSync(join(pack, "conformance.json"), JSON.stringify({ ...conformance, rules: conformance.rules.map(rule => ({ ...rule, policy: "candidates/rules/assert-used.json" })) }));
    const failing = await judge();
    expect(failing.summary.blocking.filter(row => row.rule === "assert-used")).toEqual([]);
    expect(failing.summary.promotable).toEqual([]);
    setUnits({ "clean.py": { rules: ["EX101"] } });
    const passing = await judge();
    expect(passing.summary.promotable).toEqual([expect.objectContaining({ toolPack: "valid/examplelint", rule: "assert-used", upstreamRule: "EX101", startsOn: true })]);
    const comment = renderComment(passing.summary, { commit: "0123456789abcdef" });
    expect(comment.startsWith(`${COMMENT_MARKER}\n## Conformance: `)).toBe(true);
    expect(comment).toContain("🟢 1 candidate ready to promote");
  });
});
