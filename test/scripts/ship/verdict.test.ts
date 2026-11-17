// scripts/ship/verdict.ts: the shippability bar's verdict logic. A candidate fails a metric only above 1.10 times
// main's median, fails findings set-wise (a lost true positive or a new false positive, whatever the counts), fails
// completion only where main completed, and fails coverage for every rule main ships that it cannot run.

import { describe, expect, it } from "vitest";
import { chunkSums, compareCompletion, compareFindings, compareMetric, findingKey, grouping, median, missingRules, newRuleFailures, scoreFindings, withinTolerance } from "../../../scripts/ship/verdict.js";

describe("the 10% bar", () => {
  it("passes a candidate at exactly 1.10 times main and fails one just above", () => {
    expect(withinTolerance(1000, 1100)).toBe(true);
    expect(withinTolerance(1000, 1101)).toBe(false);
    // 1.1 * 3 is 3.3000000000000003 in floating point; the bar is exact.
    expect(withinTolerance(3, 3.3)).toBe(true);
    expect(compareMetric("wall-cold", [100, 100, 100], [110, 110, 110]).status).toBe("pass");
    expect(compareMetric("wall-cold", [100, 100, 100], [111, 111, 111])).toMatchObject({ status: "fail", ratio: 1.11 });
  });

  it("compares medians, so one slow run of three does not fail the candidate", () => {
    const verdict = compareMetric("cpu-cold", [1000, 1010, 990], [1050, 5000, 1040]);
    expect(verdict).toMatchObject({ status: "pass", main: 1000, candidate: 1050 });
    expect(verdict.spreadCandidate).toBeCloseTo((5000 - 1040) / 1050);
  });

  it("marks a ratio within the run-to-run spread of the bar", () => {
    expect(compareMetric("wall-warm", [100, 90, 110], [112, 112, 112]).withinSpread).toBe(true);
    expect(compareMetric("wall-warm", [100, 100, 100], [150, 150, 150]).withinSpread).toBe(false);
  });

  it("leaves a metric uncompared when a side completed no run", () => {
    expect(compareMetric("rss-cold", [100], [])).toMatchObject({ status: "not-compared", main: 100, candidate: null });
  });
});

describe("median over N runs", () => {
  it("takes the middle run of an odd count and the mean of the middle two of an even one", () => {
    expect(median([3, 1, 2])).toBe(2);
    expect(median([5, 1, 9, 7, 3])).toBe(5);
    expect(median([4, 1, 3, 2])).toBe(2.5);
    expect(() => median([])).toThrow();
  });
});

describe("findings, set-wise", () => {
  const key = (path: string, line: number): string => findingKey("r", path, line, "X1");
  const expected = new Set([key("a.py", 1), key("a.py", 2), key("b.py", 3)]);

  it("fails a candidate that loses a true positive of main's even when its counts are equal", () => {
    const main = scoreFindings(expected, new Set([key("a.py", 1), key("a.py", 2)]), []);
    const candidate = scoreFindings(expected, new Set([key("a.py", 1), key("b.py", 3)]), []);
    expect([main.tp.size, main.fp.size, main.fn.size]).toEqual([candidate.tp.size, candidate.fp.size, candidate.fn.size]);
    expect(compareFindings(main, candidate)).toMatchObject({ status: "fail", lostTp: [key("a.py", 2)], newFp: [] });
  });

  it("fails a candidate that adds a false positive main does not have", () => {
    const main = scoreFindings(expected, new Set([key("a.py", 1), key("c.py", 9)]), []);
    const candidate = scoreFindings(expected, new Set([key("a.py", 1), key("c.py", 9), key("d.py", 4)]), []);
    expect(compareFindings(main, candidate)).toMatchObject({ status: "fail", lostTp: [], newFp: [key("d.py", 4)] });
  });

  it("passes a candidate that keeps main's true positives, finds more, and repeats only main's false positives", () => {
    const main = scoreFindings(expected, new Set([key("a.py", 1), key("c.py", 9)]), []);
    const candidate = scoreFindings(expected, new Set([key("a.py", 1), key("a.py", 2), key("c.py", 9)]), []);
    expect(compareFindings(main, candidate).status).toBe("pass");
  });

  it("applies a divergence to both sides alike; rerail-unknown explains only a miss", () => {
    const divergences = [
      { rule: "r", path: "d.py", line: 4, kind: "upstream-wrong" },
      { rule: "r", path: "b.py", line: 3, kind: "rerail-unknown" },
      { rule: "r", path: "e.py", line: 5, kind: "rerail-unknown" },
    ];
    const score = scoreFindings(expected, new Set([key("a.py", 1), key("a.py", 2), key("d.py", 4), key("e.py", 5)]), divergences);
    expect(score.explainedFp).toBe(1);
    expect(score.explainedFn).toBe(1);
    expect([...score.fp]).toEqual([key("e.py", 5)]);
    expect(score.fn.size).toBe(0);
  });
});

describe("completion", () => {
  const ok = { completed: true };
  const killed = { completed: false, reason: "killed at 2700 s (time cap 2700 s); stopped on ruff-ann003 (policy 59 of 535)" };

  it("fails a candidate that does not complete a run main completes", () => {
    expect(compareCompletion([ok, ok, ok, ok], [ok, ok, killed], 4)).toMatchObject({ status: "fail", kind: "candidate-failed", reason: killed.reason });
  });

  it("counts runs the candidate never made as not completed", () => {
    expect(compareCompletion([ok, ok], [ok], 2)).toMatchObject({ status: "fail", kind: "candidate-failed" });
  });

  it("reports both-failed, not a candidate failure, where main fails too", () => {
    expect(compareCompletion([killed], [killed], 2)).toMatchObject({ status: "pass", kind: "both-failed" });
    expect(compareCompletion([killed], [ok, ok], 2)).toMatchObject({ status: "pass", kind: "main-failed" });
  });

  it("names the rules that fail on the candidate and not on main", () => {
    const main = new Map([["a", "engine: x"]]);
    const candidate = new Map([["a", "engine: x"], ["b", "compile: y"]]);
    expect(newRuleFailures(main, candidate)).toEqual([{ rule: "b", reason: "compile: y" }]);
  });
});

describe("rules main ships that the candidate cannot run", () => {
  it("lists a rule the candidate lacks and one it has but cannot run, with the reason", () => {
    const candidate = new Map<string, string | null>([
      ["a", null],
      ["b", "check-rr reader-failed: unexpected token"],
      ["extra", null],
    ]);
    expect(missingRules(["c", "b", "a"], candidate)).toEqual([
      { rule: "b", reason: "check-rr reader-failed: unexpected token" },
      { rule: "c", reason: "no counterpart on the candidate" },
    ]);
  });

  it("lists every rule of a pack missing on the candidate", () => {
    expect(missingRules(["a", "b"], new Map())).toHaveLength(2);
  });
});

describe("a chunked .rr run", () => {
  const run = (chunk: number, rep: number, mode: "cold" | "warm", wallMs: number, completed = true) => ({ chunk, rep, mode, completed, wallMs, cpuMs: wallMs * 2, rssKB: 1000 * (chunk + 1) });

  it("sums wall and CPU over the chunks of each repetition and takes the largest chunk's peak RSS", () => {
    const sums = chunkSums([run(0, 1, "cold", 10), run(1, 1, "cold", 20), run(0, 1, "warm", 1), run(1, 1, "warm", 2)], 2);
    expect(sums).toEqual([
      { mode: "cold", rep: 1, wallMs: 30, cpuMs: 60, rssKB: 2000 },
      { mode: "warm", rep: 1, wallMs: 3, cpuMs: 6, rssKB: 2000 },
    ]);
  });

  it("leaves a chunk that failed out of every repetition, and a repetition a kept chunk did not complete", () => {
    const runs = [run(0, 1, "cold", 10), run(1, 1, "cold", 99, false), run(2, 1, "cold", 30), run(0, 2, "cold", 11)];
    expect(chunkSums(runs, 3)).toEqual([{ mode: "cold", rep: 1, wallMs: 40, cpuMs: 80, rssKB: 3000 }]);
  });
});

describe("how run-policies ran a pack", () => {
  it("counts policies run in a group of several as combined and the rest as alone", () => {
    expect(grouping([{ together: 3 }, { together: 3 }, { together: 3 }, { together: 1 }])).toEqual({ policies: 4, combined: 3, alone: 1, largest: 3 });
    expect(grouping([{ together: 1 }, { together: 1 }])).toEqual({ policies: 2, combined: 0, alone: 2, largest: 1 });
  });

  it("leaves it unknown for an engine that does not report it", () => {
    expect(grouping([{}, {}])).toBeNull();
  });
});
