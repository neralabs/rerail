// scripts/ship/metric.ts: what a cell's time is made of, reported beside the gross bar. Net time is each side's
// median less its own start-up; evaluate per rule·node divides by the rules times the nodes of the languages they
// read; a rule's multiple is its time alone, less the empty set's, over the median rule's.

import { describe, expect, it } from "vitest";
import { type CostRun, cellCost, lexicalCounts, ruleFamilies, ruleMultiples, ruleNodes, type Workload } from "../../../scripts/ship/metric.js";

const run = (mode: "cold" | "warm", wallMs: number, cpuMs: number, evaluateMs = 0, compileMs = 0): CostRun => ({ mode, completed: true, wallMs, cpuMs, phases: { load_ms: 400, compile_ms: compileMs, evaluate_ms: evaluateMs } });

const workload: Workload = {
  files: 3,
  bytes: 300,
  lines: 30,
  tokens: 120,
  nodes: 1500,
  by_language: { typescript: { files: 2, bytes: 200, lines: 20, tokens: 80, nodes: 1000 }, python: { files: 1, bytes: 100, lines: 10, tokens: 40, nodes: 500 }, json: { files: 0, bytes: 0, lines: 0, tokens: 0, nodes: null } },
  counted_by: "rerail-engine",
};

describe("net time", () => {
  it("takes each side's own start-up from its median, so a regression a start-up hides shows in the ratio", () => {
    const cost = cellCost({
      baseline: { runs: [run("cold", 760, 1000), run("cold", 770, 1010), run("cold", 750, 990)], fixed: [run("cold", 700, 950), run("cold", 710, 950)], rules: { rules: 2, by_language: { typescript: 2 } } },
      candidate: { runs: [run("cold", 830, 1100)], fixed: [run("cold", 710, 960)], rules: { rules: 2, by_language: { typescript: 2 } } },
      workload,
    });
    const wall = cost.net.find((n) => n.metric === "wall-cold")!;
    // Gross 830 / 760 = 1.09; net (830 − 710) / (760 − 705) = 2.18.
    expect(wall.baseline_ms).toBe(55);
    expect(wall.candidate_ms).toBe(120);
    expect(wall.ratio).toBeCloseTo(120 / 55);
    // No warm runs: no warm net.
    expect(cost.net.find((n) => n.metric === "wall-warm")).toMatchObject({ baseline_ms: null, ratio: null });
  });

  it("gives no ratio when the baseline's net is not positive, and none for a side without a start-up run", () => {
    const cost = cellCost({
      baseline: { runs: [run("cold", 700, 900)], fixed: [run("cold", 710, 900)], rules: { rules: 1, by_language: {} } },
      candidate: { runs: [run("cold", 800, 1000)], fixed: [], rules: { rules: 1, by_language: {} } },
      workload: null,
    });
    expect(cost.net.find((n) => n.metric === "wall-cold")).toMatchObject({ baseline_ms: -10, candidate_ms: null, ratio: null });
    expect(cost.candidate.fixed).toBeNull();
  });

  it("reports compile per rule and evaluate, less the empty set's, per rule per node read", () => {
    const cost = cellCost({
      baseline: { runs: [run("cold", 2000, 3000, 520, 300)], fixed: [run("cold", 700, 900, 20)], rules: { rules: 3, by_language: { typescript: 2, python: 1 } } },
      candidate: { runs: [run("cold", 2000, 3000, 520, 300)], fixed: [], rules: { rules: 3, by_language: { typescript: 2, python: 1 } } },
      workload,
    });
    expect(cost.baseline.compile_ms_per_rule).toBe(100);
    expect(cost.baseline.rule_nodes).toBe(2 * 1000 + 500);
    expect(cost.baseline.evaluate_ns_per_rule_node).toBeCloseTo((500 * 1e6) / 2500);
    // Without its own empty run, the whole evaluate counts.
    expect(cost.candidate.evaluate_ns_per_rule_node).toBeCloseTo((520 * 1e6) / 2500);
  });
});

describe("the nodes a rule set reads", () => {
  it("maps a rule's languages to the families the engine parses, splitting a rule over the families it reads", () => {
    expect(ruleFamilies('{"clauses":[{"define":{"language":"javascript"}},{"language":"tsx"},{"languageOptions":{}}]}')).toEqual(["typescript"]);
    expect(ruleFamilies('{"clauses":[{"language":"python"},{"language":"typescript"}]}')).toEqual(["python", "typescript"]);
    expect(ruleNodes({ rules: 1, by_language: { python: 0.5, typescript: 0.5 } }, workload)).toBe(750);
  });

  it("charges a rule without a language every parsed node, and gives no count without node counts", () => {
    expect(ruleNodes({ rules: 2, by_language: { "*": 1, python: 1 } }, workload)).toBe(1500 + 500);
    expect(ruleNodes({ rules: 1, by_language: { python: 1 } }, { ...workload, nodes: null })).toBeNull();
  });

  it("counts a last line without a newline, and every token of a generic lexer", () => {
    expect(lexicalCounts("a = 1;\nb('x y')")).toEqual({ lines: 2, tokens: 8 });
    expect(lexicalCounts("one\n")).toEqual({ lines: 1, tokens: 1 });
    expect(lexicalCounts("")).toEqual({ lines: 0, tokens: 0 });
  });
});

describe("the rule cost multiple", () => {
  it("divides each rule's time less the empty set's by the median rule's, per side", () => {
    const multiples = ruleMultiples(
      [
        { rule: "a", baseline_ms: 150, candidate_ms: 160 },
        { rule: "b", baseline_ms: 200, candidate_ms: 210 },
        { rule: "shadow", baseline_ms: 750, candidate_ms: 2260 },
      ],
      { baseline_ms: 50, candidate_ms: 60 },
    );
    expect(multiples.get("a")).toEqual({ multiple: { baseline: 2 / 3, candidate: 2 / 3 }, flags: [] });
    expect(multiples.get("shadow")!.multiple).toEqual({ baseline: 700 / 150, candidate: 2200 / 150 });
    // Above 10× the median rule, and grown from 4.7× to 14.7×: both flags.
    expect(multiples.get("shadow")!.flags).toEqual(["14.7× the pack's median rule", "multiple grew 4.7× → 14.7×"]);
  });

  it("flags growth only above 1.5 times, and leaves a side that could not run a rule without a multiple", () => {
    const multiples = ruleMultiples(
      [
        { rule: "a", baseline_ms: 100, candidate_ms: 100 },
        { rule: "b", baseline_ms: 100, candidate_ms: 150 },
        { rule: "c", baseline_ms: 100, candidate_ms: null },
      ],
      { baseline_ms: null, candidate_ms: null },
    );
    expect(multiples.get("b")).toEqual({ multiple: { baseline: 1, candidate: 1.2 }, flags: [] });
    expect(multiples.get("c")!.multiple).toEqual({ baseline: 1, candidate: null });
  });
});
