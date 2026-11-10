/**
 * The gate (conformance/languages/gates.ts): every port of a gated tool-pack through the conformance checks, judged on
 * the pack's conformance report and gates.yaml's declared facts, and the summary and sticky comment built from them.
 * It runs neither `loads` nor `compact`: pack validation's check 1 does (test/engine/catalog/packs.test.ts).
 */
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { COMMENT_MARKER, renderComment } from "../../conformance/languages/comment.js";
import { BLOCKING_GATES, judgeToolPack } from "../../conformance/languages/gates.js";
import { fixturesDirectory, fixturesExpectedPath, loadPack } from "../../conformance/languages/pack.js";
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
  const verdict = judgeToolPack(root, loaded, report, "examplelint");
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
    expect(after.verdict.checks.map(check => [check.id, check.ok])).toEqual([["declared-pins", true], ["declared-lints", true], ["conformance-runs", true]]);
  });

  it("report a shipped document with no conformance.json entry as blocking, once", async () => {
    const { verdict, summary } = await judge();
    expect(verdict.unmapped).toEqual([{ file: "packs/valid/rules/pass-statement.json", location: "shipped" }]);
    expect(summary.blocking.filter(row => row.stoppedAt === "Mapping")).toEqual([
      { pack: "valid", rule: "packs/valid/rules/pass-statement.json", stoppedAt: "Mapping", check: "conformance-entry", why: "not mapped to an upstream rule: add its conformance.json entry" },
    ]);
  });

  it("leave a pack's own rule (conformance.json own) out of the unmapped documents and the tool-pack's rules, and give it no verdict", async () => {
    const path = join(pack, "conformance.json");
    writeFileSync(path, JSON.stringify({ ...JSON.parse(readFileSync(path, "utf8")), own: ["pass-statement"] }));
    const { verdict, summary } = await judge();
    expect(verdict.unmapped).toEqual([]);
    expect(verdict.rules.map(rule => rule.id)).toEqual(["assert-used"]);
    expect(summary.blocking.filter(row => row.stoppedAt === "Mapping")).toEqual([]);
    expect(summary.toolPacks.map(entry => entry.toolPack)).toEqual(["valid/examplelint"]);
    expect(summary.shipped.rules).toBe(1);
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

  it("run neither loads nor compact: a port whose document fails both still passes here", async () => {
    setUnits({ "clean.py": { rules: ["EX101"] } });
    // gates.yaml says the tool lints .pyi files the port's scope leaves out (loads, in-scope), and the document writes
    // a product out case by case (compact): pack validation's check 1 fails it on both, and this gate reads neither.
    writeFileSync(join(pack, "upstream", "examplelint", "gates.yaml"), GATES("{ config: default, options: [] }").replace('lints: [".py"]', 'lints: [".py", ".pyi"]'));
    const cells = ["print", "open", "input", "eval", "exec"].flatMap(callee =>
      ["append", "extend", "insert", "remove", "update", "setdefault"].flatMap(method =>
        ["class_definition", "function_definition", "lambda", "for_statement", "with_statement"].map(enclosing => ({ kind: "call", regex: `^${callee}\\(.*\\)\\.${method}\\b`, inside: { kind: enclosing, stopBy: "end" } })),
      ),
    );
    const document = join(pack, "rules", "assert-used.json");
    writeFileSync(document, JSON.stringify({
      ...(JSON.parse(readFileSync(document, "utf8")) as Record<string, unknown>),
      scope: { include: ["**/*.py"], exclude: [] },
      clauses: [{ id: "assert-used", forbid: { ast: { language: "python", rule: { any: [{ kind: "assert_statement" }, ...cells] } } }, violation: { anchor: "witness.span", code: "EX101", message: "Replace this assert statement with an explicit check." } }],
    }));
    const { verdict, summary } = await judge();
    expect(BLOCKING_GATES.map(gate => gate.id)).toEqual(["conformance"]);
    expect(Object.keys(verdict.rules[0]!.gates)).toEqual(["upstream-status", "conformance"]);
    expect(verdict.checks.map(check => check.id)).not.toContain("bundled-load");
    expect(Object.keys(summary.candidateFunnel["valid/examplelint"]!)).toEqual(["Passes the conformance checks", "Tool-pack checks", "passes every gate"]);
    // No gate column or failure row of either: the comment only says where they are checked.
    expect(renderComment(summary)).not.toMatch(/Compiles and loads|Compact \(|in-scope|current-contract/u);
    expect(verdict.rules[0]).toMatchObject({ id: "assert-used", pass: true, stoppedAt: null });
    expect(summary.blocking.filter(row => row.rule === "assert-used")).toEqual([]);
  });

  it("fail every rule when a port declares a file type the tool does not lint", async () => {
    setUnits({ "clean.py": { rules: ["EX101"] } });
    const path = join(pack, "conformance.json");
    const conformance = JSON.parse(readFileSync(path, "utf8")) as { rules: Record<string, unknown>[] };
    writeFileSync(path, JSON.stringify({ ...conformance, rules: conformance.rules.map(rule => ({ ...rule, lints: [".py", ".rb"] })) }));
    expect((await judge()).verdict.rules[0]).toMatchObject({ pass: false, stoppedAt: "tool-pack", failure: { check: "declared-lints", why: expect.stringContaining("assert-used .rb") } });
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
    const runs = (errors: string[]) => judgeToolPack(root, loaded, { ...report, errors }, "examplelint").checks.find(check => check.id === "conformance-runs");
    expect(runs(["samples otherlint: x-app-py-000000000000/app.py.labels.json: not JSON"])).toMatchObject({ ok: true });
    expect(runs(["fixtures otherlint: notes.txt is not a python source by the host's routing"])).toMatchObject({ ok: true });
    expect(runs(["samples examplelint: x-app-py-000000000000/app.py.labels.json: not JSON"])).toMatchObject({ ok: false });
    expect(runs(["stale exemption: capability syntax is covered by rule(s) assert-used"])).toMatchObject({ ok: false });
  });
});

describe("a pack's families", () => {
  it("score fixtures in every family the pack names, and refuse one of another family", async () => {
    const loaded = loadPack(pack);
    writeFileSync(join(fixturesDirectory(loaded, "examplelint"), "clean.js"), "export const total = 1;\n");
    const expectedPath = fixturesExpectedPath(loaded, "examplelint");
    const expected = JSON.parse(readFileSync(expectedPath, "utf8")) as { source: { files: string[] } };
    writeFileSync(expectedPath, JSON.stringify({ ...expected, source: { ...expected.source, files: [...expected.source.files, "clean.js"] } }));
    const errors = async () => (await runPack(loadPack(pack), { fixtures: true, samples: false, date: "2026-01-01" })).errors;
    expect(await errors()).toContain("fixtures examplelint: clean.js is not a python source by the host's routing");
    const path = join(pack, "conformance.json");
    writeFileSync(path, JSON.stringify({ ...JSON.parse(readFileSync(path, "utf8")), family: ["python", "ecma"] }));
    expect(await errors()).toEqual([]);
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

  it("puts a shrinks check's list path in the why, not the Pack column", () => {
    const detail = "1 entr(ies) the base lacks (plain/b): the list may only shrink";
    expect(ungatedRow({ id: "shrinks:conformance/languages/ungated-rules.json", ok: false, detail })).toEqual({
      pack: "-", rule: "-", stoppedAt: "Not gated / pending", check: "shrinks", why: `conformance/languages/ungated-rules.json: ${detail}`,
    });
    expect(ungatedRow({ id: "pending:plain/lintx", ok: false, detail: "x" })).toMatchObject({ pack: "plain/lintx", check: "pending" });
  });
});
