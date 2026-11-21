// scripts/ship/verdict.ts: the shippability bar's verdict logic. A candidate fails a metric only above 1.15 times
// main's median, fails findings set-wise (a lost true positive or a new false positive, whatever the counts), fails
// completion only where main completed, and fails coverage for every rule main ships that it cannot run. Slice B counts
// a rule over the bar only when it is still over after confirmation and moved by at least 20 ms; slice C picks the
// largest, median and 5th-percentile file; every failing figure names the phase that moved.

import { describe, expect, it } from "vitest";
import {
  chunkSums,
  compareCompletion,
  compareFindings,
  compareMetric,
  findingKey,
  grouping,
  judgeTime,
  locate,
  median,
  missingRules,
  needsConfirmation,
  newRuleFailures,
  percentile,
  type PhaseRun,
  pickFiles,
  ruleSliceStats,
  scoreFindings,
  SLICES_TOLERANCE_PERCENT,
  TOLERANCE_PERCENT,
  withinTolerance,
} from "../../../scripts/ship/verdict.js";

describe("the bars", () => {
  it("passes a slice figure at exactly 1.15 times main and fails one just above", () => {
    const bar = SLICES_TOLERANCE_PERCENT;
    expect(bar).toBe(15);
    expect(withinTolerance(1000, 1150, bar)).toBe(true);
    expect(withinTolerance(1000, 1151, bar)).toBe(false);
    // 3 * 1.15 is 3.4499999999999997 in floating point; the bar is exact.
    expect(withinTolerance(3, 3.45, bar)).toBe(true);
    expect(compareMetric("wall-cold", [100, 100, 100], [115, 115, 115], bar).status).toBe("pass");
    expect(compareMetric("wall-cold", [100, 100, 100], [116, 116, 116], bar)).toMatchObject({ status: "fail", ratio: 1.16 });
    // What fails the waits' 1.10 bar passes the slices'.
    expect(compareMetric("wall-cold", [100, 100, 100], [112, 112, 112], bar).status).toBe("pass");
  });

  it("keeps the waits at 1.10 times main, the default", () => {
    expect(TOLERANCE_PERCENT).toBe(10);
    expect(withinTolerance(1000, 1100)).toBe(true);
    expect(withinTolerance(1000, 1101)).toBe(false);
    // 1.1 * 3 is 3.3000000000000003 in floating point; the bar is exact.
    expect(withinTolerance(3, 3.3)).toBe(true);
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

describe("a chunked .rerail run", () => {
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

describe("slice B: a rule alone", () => {
  it("passes a rule within the bar at its first runs, whatever it moved", () => {
    expect(judgeTime("r", [100, 100], [115, 115])).toMatchObject({ status: "pass", overBefore: false });
  });

  it("reports a rule over at its first runs and within at its confirmation runs as noisy", () => {
    const v = judgeTime("r", [100, 100], [150, 150], [100, 100, 100], [110, 110, 110]);
    expect(v).toMatchObject({ status: "noisy", overBefore: true, finalRatio: 1.1, finalDeltaMs: 10 });
  });

  it("counts a rule still over after confirmation that moved by at least 20 ms", () => {
    const v = judgeTime("r", [100, 100], [150, 150], [100, 100, 100], [140, 140, 140]);
    expect(v).toMatchObject({ status: "fail", overBefore: true, ratio: 1.5, confirm: { main: 100, candidate: 140, ratio: 1.4 }, finalDeltaMs: 40 });
    // Exactly the floor counts.
    expect(judgeTime("r", [100], [130], [100], [120]).status).toBe("fail");
  });

  it("does not count a rule still over after confirmation that moved by less than 20 ms, and keeps it in the unfiltered list", () => {
    const v = judgeTime("tiny", [4, 4], [12, 12], [4, 4, 4], [10, 10, 10]);
    expect(v).toMatchObject({ status: "under-floor", overBefore: true, finalRatio: 2.5, finalDeltaMs: 6 });
    const stats = ruleSliceStats([v]);
    expect(stats.overBefore).toEqual(["tiny"]);
    expect(stats.failing).toEqual([]);
    expect(stats.overAlert).toEqual([]);
  });

  it("judges the floor on the confirmation runs, not the first ones", () => {
    // Over by 30 ms at first; confirmed over, but by 17 ms only.
    expect(judgeTime("r", [100, 100], [130, 130], [100, 100, 100], [117, 117, 117]).status).toBe("under-floor");
  });

  it("lets an unconfirmed rule over by at least the floor stand, and confirms only those that could count", () => {
    const big = judgeTime("big", [100, 100], [200, 200]);
    expect(big.status).toBe("fail");
    expect(needsConfirmation(big)).toBe(true);
    expect(needsConfirmation(judgeTime("tiny", [4, 4], [12, 12]))).toBe(false);
    expect(needsConfirmation(judgeTime("within", [100, 100], [110, 110]))).toBe(false);
    expect(needsConfirmation(judgeTime("done", [100, 100], [200, 200], [100], [200]))).toBe(false);
  });

  it("leaves a rule a side did not time uncompared", () => {
    expect(judgeTime("r", [100], [])).toMatchObject({ status: "not-compared", overBefore: false });
  });

  it("summarises a pack's rules: median and p95 ratio, counted, over 1.5×, and the worst five", () => {
    const rules = [
      judgeTime("a", [100], [100]),
      judgeTime("b", [100], [105]),
      judgeTime("c", [100], [110]),
      judgeTime("d", [100], [200], [100], [190]),
      judgeTime("e", [10], [30]),
      judgeTime("f", [100], [120], [100], [101]),
      judgeTime("g", [100], []),
    ];
    const stats = ruleSliceStats(rules);
    expect(stats).toMatchObject({ rules: 7, compared: 6, p95Ratio: 3, overBefore: ["d", "e", "f"], failing: ["d", "e"], overAlert: ["d", "e"] });
    expect(stats.medianRatio).toBeCloseTo(1.15);
    expect(stats.worst.map((w) => [w.rule, w.ratio, w.main, w.candidate])).toEqual([
      ["e", 3, 10, 30],
      ["d", 1.9, 100, 190],
      ["c", 1.1, 100, 110],
      ["b", 1.05, 100, 105],
      ["f", 1.01, 100, 101],
    ]);
  });

  it("takes the nearest-rank percentile", () => {
    expect(percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 95)).toBe(10);
    expect(percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 50)).toBe(5);
    expect(percentile([7], 95)).toBe(7);
  });
});

describe("the phase that moved", () => {
  const run = (mode: "cold" | "warm", wallMs: number, load: number, compile: number, evaluate: number, rssKB = 100_000): PhaseRun => ({ mode, completed: true, wallMs, rssKB, phases: { load_ms: load, compile_ms: compile, evaluate_ms: evaluate } });

  it("names the phase whose median grew most, with every phase's change", () => {
    const main = [run("cold", 1000, 300, 100, 400), run("cold", 1000, 300, 100, 400)];
    const candidate = [run("cold", 1500, 300, 110, 880), run("cold", 1500, 300, 110, 880)];
    expect(locate("wall-cold", main, candidate)).toBe("evaluate: load +0 ms, compile +10 ms, evaluate +480 ms, startup +10 ms");
  });

  it("names startup when the wall outside the phases grew", () => {
    expect(locate("cpu-cold", [run("cold", 1000, 300, 100, 400)], [run("cold", 1400, 300, 100, 400)])).toMatch(/^startup: /);
  });

  it("names the store for a warm figure over the bar whose cold counterpart passes", () => {
    expect(locate("wall-warm", [run("warm", 500, 300, 100, 50)], [run("warm", 800, 300, 100, 350)], { coldPasses: true })).toMatch(/^store: over only on the reused store/);
  });

  it("limits a rule alone to compile and evaluate", () => {
    expect(locate("wall-cold", [run("cold", 1000, 300, 10, 20)], [run("cold", 1300, 600, 60, 20)], { only: ["compile", "evaluate"] })).toBe("compile: compile +50 ms, evaluate +0 ms");
  });

  it("splits peak RSS between the empty rule set (startup) and the rules", () => {
    const main = [run("cold", 1000, 300, 100, 400, 200_000)];
    const candidate = [run("cold", 1000, 300, 100, 400, 300_000)];
    const fixed = (rss: number): PhaseRun[] => [run("cold", 600, 300, 0, 10, rss)];
    expect(locate("rss-cold", main, candidate, { fixed: { main: fixed(100_000), candidate: fixed(190_000) } })).toMatch(/^startup: the empty rule set's peak RSS \+88 MB of \+98 MB/);
    expect(locate("rss-cold", main, candidate, { fixed: { main: fixed(100_000), candidate: fixed(110_000) } })).toMatch(/^evaluate: the empty rule set's peak RSS \+10 MB of \+98 MB/);
  });
});

describe("slice C: three files of the real-code corpus", () => {
  it("picks the largest, the median and the 5th percentile by bytes among the pack's source extensions", () => {
    const files = Array.from({ length: 40 }, (_, i) => ({ path: `src/f${String(i).padStart(2, "0")}.py`, bytes: (i + 1) * 100 }));
    files.push({ path: "README.md", bytes: 1_000_000 }, { path: "setup.cfg", bytes: 5 });
    expect(pickFiles("python", files)).toEqual([
      { label: "largest", path: "src/f39.py", bytes: 4000 },
      { label: "median", path: "src/f20.py", bytes: 2100 },
      { label: "small", path: "src/f02.py", bytes: 300 },
    ]);
  });

  it("leaves Blade templates to the blade pack, and breaks ties by path", () => {
    const files = [
      { path: "b.php", bytes: 10 },
      { path: "a.php", bytes: 10 },
      { path: "view.blade.php", bytes: 99 },
    ];
    expect(pickFiles("php", files).map((f) => f.path)).toEqual(["b.php", "b.php", "a.php"]);
    expect(pickFiles("blade", files).map((f) => f.path)).toEqual(["view.blade.php", "view.blade.php", "view.blade.php"]);
  });

  it("refuses a pack with no declared source extensions", () => {
    expect(() => pickFiles("cobol", [])).toThrow(/no source extensions declared for pack cobol/);
  });
});
