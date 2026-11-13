/**
 * The conformance gate's floor for a shipped port (`reportFailures`, conformance/languages/runner.ts): divergences can
 * excuse every finding of a rule's fixtures, so a port that matches nothing still scores 0 FP and 0 FN. Such a port
 * fails unless pack.json gives its rule examples, which the pack checks prove fire. A candidate fails nothing, but its
 * fixtures expected findings must still cover it.
 */
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { loadPack, type Divergence } from "../../conformance/languages/pack.js";
import { reportFailures, runPack } from "../../conformance/languages/runner.js";
import { fixturesPath } from "../support/oracles.js";

const OPTIONS = { fixtures: true, samples: false, date: "2026-01-01" };

const editJson = (path: string, edit: (value: Record<string, unknown>) => unknown): void =>
  writeFileSync(path, JSON.stringify(edit(JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>), null, 2));

describe("a shipped port with no fixture TP", () => {
  let directory: string;

  beforeEach(() => {
    // The `valid` pack ports examplelint's EX101 (every assert) as rules/assert-used.json, without pack.json examples.
    directory = mkdtempSync(join(tmpdir(), "rerail-port-proof-"));
    cpSync(fixturesPath("packs", "valid"), directory, { recursive: true });
    // The port now matches nothing (the fixtures have no `global`), and not-comparable divergences excuse both
    // upstream findings: the run has no FP, no FN and nothing stale.
    editJson(join(directory, "rules", "assert-used.json"), document => {
      const [clause] = document.clauses as [{ forbid: { ast: { rule: { kind: string } } } }];
      clause.forbid.ast.rule.kind = "global_statement";
      return document;
    });
    const excused = (line: number): Divergence => ({
      rule: "assert-used",
      source: "fixtures",
      path: "asserts.py",
      line,
      kind: "not-comparable",
      evidence: "a blanket divergence covering every finding line of the fixture",
    });
    writeFileSync(join(directory, "divergences.json"), JSON.stringify([excused(1), excused(3)], null, 2));
  });

  afterEach(() => rmSync(directory, { recursive: true, force: true }));

  it("fails the gate when pack.json gives its rule no examples", async () => {
    const report = await runPack(loadPack(directory), OPTIONS);
    expect(report.rules.map(({ rule, tp, fp, fn, examples }) => ({ rule, tp, fp, fn, examples }))).toEqual([
      { rule: "assert-used", tp: 0, fp: 0, fn: 0, examples: false },
    ]);
    expect(reportFailures(report)).toEqual(["assert-used (fixtures): 0 TP and no examples in pack.json; nothing shows it catches anything"]);
    expect(report.pass).toBe(false);
  });

  it("passes the gate once pack.json gives its rule examples", async () => {
    editJson(join(directory, "pack.json"), pack => ({
      ...pack,
      rules: (pack.rules as Record<string, unknown>[]).map(rule =>
        rule.document === "rules/assert-used.json"
          ? { ...rule, examples: { caught: { path: "src/a.py", text: "assert ready\n" }, passes: { path: "src/a.py", text: "ready = True\n" } } }
          : rule,
      ),
    }));
    const report = await runPack(loadPack(directory), OPTIONS);
    expect(report.rules.map(({ tp, examples }) => ({ tp, examples }))).toEqual([{ tp: 0, examples: true }]);
    expect(reportFailures(report)).toEqual([]);
  });

  it("does not fail a candidate port", async () => {
    editJson(join(directory, "conformance.json"), pack => ({
      ...pack,
      rules: (pack.rules as Record<string, unknown>[]).map(rule => ({ ...rule, policy: "candidates/assert-used.json" })),
    }));
    cpSync(join(directory, "rules", "assert-used.json"), join(directory, "candidates", "assert-used.json"));
    const report = await runPack(loadPack(directory), OPTIONS);
    expect(report.rules.map(({ tp, candidate }) => ({ tp, candidate }))).toEqual([{ tp: 0, candidate: true }]);
    expect(reportFailures(report)).toEqual([]);
  });

  it("is still an error for a candidate the fixtures expected findings do not cover", async () => {
    editJson(join(directory, "conformance.json"), pack => ({
      ...pack,
      rules: (pack.rules as Record<string, unknown>[]).map(rule => ({ ...rule, policy: "candidates/assert-used.json" })),
    }));
    cpSync(join(directory, "rules", "assert-used.json"), join(directory, "candidates", "assert-used.json"));
    editJson(join(directory, "upstream", "examplelint", "fixtures.expected.json"), expected => ({
      ...expected,
      source: { ...(expected.source as Record<string, unknown>), rules: ["EX999"] },
      findings: [],
    }));
    const report = await runPack(loadPack(directory), OPTIONS);
    expect(report.errors).toEqual(["assert-used (fixtures): the examplelint expected findings do not cover EX101"]);
    expect(report.pass).toBe(false);
  });
});
