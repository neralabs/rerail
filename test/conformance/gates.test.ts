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
import { summarize, ungatedRow } from "../../conformance/languages/summary.js";
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
  const expected = JSON.parse(readFileSync(path, "utf8")) as { source: Record<string, unknown> };
  writeFileSync(path, JSON.stringify({ ...expected, source: { ...expected.source, units } }));
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

describe("the conformance report's errors", () => {
  it("fail only the tool-pack whose fixtures or samples they name; an error naming no tool fails every tool-pack", async () => {
    setUnits({ "clean.py": { rules: ["EX101"] } });
    const path = join(pack, "conformance.json");
    const conformance = JSON.parse(readFileSync(path, "utf8")) as { tools: unknown[] };
    writeFileSync(path, JSON.stringify({ ...conformance, tools: [...conformance.tools, { name: "otherlint", version: "1.0.0" }] }));
    const loaded = loadPack(pack);
    const report = await runPack(loaded, { fixtures: true, samples: false, date: "2026-01-01" });
    const runs = (errors: string[]) => judgeToolPack(root, loadEngine(), loaded, { ...report, errors }, "examplelint").checks.find(check => check.id === "conformance-runs");
    expect(runs(["samples otherlint: x-app-py-000000000000/app.py.labels.json: not JSON"])).toMatchObject({ ok: true });
    expect(runs(["fixtures otherlint: notes.txt is not a python source by the host's routing"])).toMatchObject({ ok: true });
    expect(runs(["samples examplelint: x-app-py-000000000000/app.py.labels.json: not JSON"])).toMatchObject({ ok: false });
    expect(runs(["stale exemption: capability syntax is covered by rule(s) assert-used"])).toMatchObject({ ok: false });
  });
});

describe("the summary", () => {
  it("keeps a report failure line of a shipped rule the gate passed, so a red job never reads green", async () => {
    setUnits({ "clean.py": { rules: ["EX101"] } });
    const { verdict } = await judge();
    expect(verdict.rules[0]).toMatchObject({ id: "assert-used", pass: true });
    const line = `assert-used (fixtures): 1 unexplained FP on rename variants, first {"path":"renamed/asserts.py","line":5,"column":5}`;
    const summary = summarize([{ pack: "valid", failures: [line] }], [verdict], UNGATED);
    expect(summary.blocking).toContainEqual({ pack: "valid", rule: "-", stoppedAt: "Conformance", check: "report", why: line });
    expect(summary.pass).toBe(false);
    expect(renderComment(summary)).not.toContain("Nothing blocks.");
  });

  it("lists a shipped rule passing a check vacuously, without blocking, and the comment shows it", async () => {
    setUnits({ "clean.py": { rules: ["EX101"] } });
    const { verdict } = await judge();
    const vacuous = { id: "holds-under-rename", ok: true, vacuous: true as const, detail: "vacuous: no finding of the rule survives on its 2 rename variant unit(s)" };
    const rule = verdict.rules[0]!;
    const withVacuous = { ...verdict, rules: [{ ...rule, gates: { ...rule.gates, conformance: [...rule.gates.conformance.filter(check => check.id !== vacuous.id), vacuous] } }] };
    const summary = summarize([], [withVacuous], UNGATED);
    expect(summary.vacuous).toEqual([{ toolPack: "valid/examplelint", rule: "assert-used", check: "holds-under-rename", why: vacuous.detail }]);
    expect(summary.blocking.filter(row => row.rule === "assert-used")).toEqual([]);
    expect(renderComment(summary)).toContain("### Passing without evidence");
  });

  it("puts a shrinks check's list path in the why, not the Pack column", () => {
    const detail = "1 entr(ies) the base lacks (plain/b): the list may only shrink";
    expect(ungatedRow({ id: "shrinks:conformance/languages/ungated-rules.json", ok: false, detail })).toEqual({
      pack: "-", rule: "-", stoppedAt: "Not gated / pending", check: "shrinks", why: `conformance/languages/ungated-rules.json: ${detail}`,
    });
    expect(ungatedRow({ id: "pending:plain/lintx", ok: false, detail: "x" })).toMatchObject({ pack: "plain/lintx", check: "pending" });
  });
});
