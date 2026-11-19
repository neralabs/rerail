// scripts/ship/report-model.ts: what the ship / no-ship report counts. Every pack is measured in three slices (A all
// rules × all files, B each rule alone, C all rules × one file); a pack missing one is incomplete and never passes, and a
// run narrowed by --packs or --corpus is PARTIAL, never PASS. A figure failing at the median counts only when its
// confirmation runs fail too (else it is noisy, reported and not counted); a B rule counts only when still over after
// confirmation and moved by at least 20 ms; a slowdown is attributed to the rules that carry it; every failing item
// names the phase that moved.

import { describe, expect, it } from "vitest";
import { LINUX_METHOD } from "../../../scripts/ship/platform.js";
import { attribute, buildReport, metricStatus, ruleHint, type RunMeta, SCHEMA } from "../../../scripts/ship/report-model.js";
import { renderReport } from "../../../scripts/ship/report.js";
import type { CellVerdict, Corpus, MeasuredMetric, OneFileVerdict, OneRuleTiming, OneRuleVerdict, PackVerdict, RuleTiming, Verdict } from "../../../scripts/ship/shipbar.js";
import { compareMetric, type FileLabel, judgeTime, SLICES_TOLERANCE_PERCENT } from "../../../scripts/ship/verdict.js";

function measured(metric: string, main: number[], candidate: number[], confirm?: [number[], number[]], locator: string | null = null): MeasuredMetric {
  return {
    verdict: compareMetric(metric, main, candidate, SLICES_TOLERANCE_PERCENT),
    mainRuns: main,
    candidateRuns: candidate,
    confirm: confirm === undefined ? null : { verdict: compareMetric(metric, confirm[0], confirm[1], SLICES_TOLERANCE_PERCENT), mainRuns: confirm[0], candidateRuns: confirm[1] },
    locator,
  };
}

const completed = { status: "pass" as const, kind: "both-completed" as const, reason: "" };

/** A slice B rule alone: compile + evaluate per side, its confirmation runs where given; same findings. */
function rule(name: string, main: number[], candidate: number[], confirm: [number[], number[]] = [[], []], locator: string | null = null): OneRuleTiming {
  return {
    rule: name,
    times: [judgeTime("compile+evaluate", main, candidate, confirm[0], confirm[1])],
    locator,
    completion: completed,
    findings: { both: 3, mainOnly: 0, candidateOnly: 0, samples: [] },
    ruleFailure: null,
    wallMs: { main: 900, candidate: 900 },
    rssKB: { main: 150_000, candidate: 150_000 },
  };
}

function oneRule(corpus: Corpus, rules: OneRuleTiming[] = [rule("a", [50, 50], [50, 50])]): OneRuleVerdict {
  const fixed: OneRuleTiming = { ...rule("(none)", [], []), times: [judgeTime("wall-cold", [800, 800], [810, 810]), judgeTime("load", [500, 500], [505, 505]), judgeTime("compile", [0, 0], [0, 0]), judgeTime("evaluate", [40, 40], [41, 41])], findings: null };
  return { corpus, note: "", unmeasured: [], runs: 2, fixed, rules, load: { runs: 4 * (rules.length + 1), loadMedian: 0.5, loadMax: 1.25, siblingBusyMax: 0.04 } };
}

function oneFile(label: FileLabel, metrics: MeasuredMetric[] = [measured("wall-cold", [700, 700, 700], [700, 700, 700])], over: Partial<OneFileVerdict> = {}): OneFileVerdict {
  return {
    label,
    path: `src/${label}.ts`,
    bytes: 2048,
    note: "",
    completion: { ...completed, stoppedOn: null },
    metrics,
    findings: { both: 4, mainOnly: 0, candidateOnly: 0, samples: [] },
    ruleFailures: [],
    load: { main: [0.2], candidate: [0.3] },
    siblingBusyMax: 0.02,
    runs: { main: 6, candidate: 6 },
    ...over,
  };
}

const threeFiles = (): OneFileVerdict[] => [oneFile("largest"), oneFile("median"), oneFile("small")];

function cell(corpus: Corpus, metrics: MeasuredMetric[]): CellVerdict {
  return {
    corpus,
    note: "",
    blocked: null,
    completion: { ...completed, stoppedOn: null },
    ruleFailures: [],
    metrics,
    findings: null,
    agreement: { both: 10, mainOnly: 0, candidateOnly: 0, samples: [] },
    load: { main: [0.1], candidate: [0.1] },
    siblingBusyMax: 0,
    methods: [LINUX_METHOD],
    runs: { main: 6, candidate: 6 },
    rules: { main: 3, candidate: 3 },
    cost: null,
    caps: { memCap: ["24G"], timeCapS: [2700] },
    chunked: null,
    progress: null,
    grouping: null,
    fallback: [],
  };
}

/** A pack measured in all three slices: B on each A corpus, C's three files where real code is in scope. */
function pack(name: string, cells: CellVerdict[], over: Partial<PackVerdict> = {}): PackVerdict {
  return { pack: name, noBaseline: false, missing: [], extra: [], cells, oneRule: cells.map((c) => oneRule(c.corpus)), oneFile: cells.some((c) => c.corpus === "real") ? threeFiles() : [], slicesNote: null, ...over };
}

function verdict(packs: PackVerdict[], partial: string[] = []): Verdict {
  return {
    tolerancePercent: 15,
    runs: 3,
    baseline: { dir: "/b", sha: "b".repeat(40), ref: "v0.1.4" },
    candidate: { dir: "/c", sha: "c".repeat(40) },
    corpora: { baseline: "b".repeat(40), fixtures: {}, real: {} },
    machine: { hosts: ["h"], cpus: ["8,9,10,11"], threads: [4], memCap: ["24G"], timeCapS: [2700], methods: [{ method: LINUX_METHOD, runs: 12 }] },
    machines: [],
    candidates: [{ candidate: "ships", description: "as it ships", packs }],
    gate: ["ships"],
    partial,
  };
}

const meta: RunMeta = { startedAt: null, finishedAt: "2026-11-17T00:00:00Z", invokedBy: null, command: null, harnessSha: "h", confirmRuns: 3, ruleRuns: 2, fileRuns: 3 };
const passing = (): MeasuredMetric[] => [measured("wall-cold", [700, 710, 720], [700, 710, 720])];

describe("confirmation", () => {
  it("counts a failing metric only when its confirmation runs fail too, else reports it as noise", () => {
    expect(metricStatus(measured("wall-cold", [700, 710, 720], [850, 860, 870], [[700, 705, 710], [740, 745, 750]]))).toBe("noisy");
    expect(metricStatus(measured("wall-cold", [700, 710, 720], [850, 860, 870], [[700, 705, 710], [850, 855, 860]]))).toBe("fail");
  });

  it("keeps a failure that was never confirmed", () => {
    expect(metricStatus(measured("wall-cold", [700, 710, 720], [850, 860, 870]))).toBe("fail");
  });
});

describe("attributing a slowdown to rules", () => {
  const attributed = (rules: RuleTiming[]) => {
    const { rules: items, totalAddedMs } = attribute(rules);
    return { rules: items, total_added_ms: totalAddedMs };
  };

  it("names one rule alone when it carries at least half of the added time", () => {
    const a = attributed([
      { rule: "no-shadow", main: 3700, candidate: 12100 },
      { rule: "member-ordering", main: 520, candidate: 690 },
      { rule: "no-redeclare", main: 210, candidate: 290 },
    ]);
    expect(a.rules[0]?.rule).toBe("no-shadow");
    expect(ruleHint(a)).toBe("`no-shadow` +8.40 s of +8.65 s");
  });

  it("reports a regression spread over every rule, not one culprit", () => {
    expect(ruleHint(attributed(["a", "b", "c", "d"].map((rule) => ({ rule, main: 100, candidate: 120 }))))).toMatch(/^spread over 4 rules: \+20 ms each \(median\)/);
  });

  it("puts a rule one side could not run last, with no added time", () => {
    const a = attributed([
      { rule: "broken", main: 100, candidate: null },
      { rule: "slower", main: 100, candidate: 300 },
    ]);
    expect(a.rules.map((r) => [r.rule, r.added_ms])).toEqual([
      ["slower", 200],
      ["broken", null],
    ]);
  });
});

describe("the report", () => {
  it("is schema v2 with the 1.15 bar and the slices' repetitions", () => {
    const report = buildReport(verdict([pack("sql", [cell("real", [measured("wall-cold", [1000], [1120])])])]), meta);
    expect(report.schema).toBe(SCHEMA);
    expect(SCHEMA).toBe("ship-no-ship/report/v2");
    expect(report.bar).toMatchObject({ max_ratio: 1.15, rule_runs: 2, file_runs: 3, confirm_runs: 3, rule_floor_ms: 20, rule_alert_ratio: 1.5 });
    // 1.12× failed the old 1.10 bar; it passes 1.15.
    expect(report.verdict.status).toBe("PASS");
  });

  it("passes a candidate whose only failure is noise, and lists the noise apart", () => {
    const report = buildReport(verdict([pack("sql", [cell("real", [measured("wall-cold", [700, 710, 720], [850, 860, 870], [[700, 705, 710], [740, 745, 750]])])])]), meta);
    expect(report.verdict.status).toBe("PASS");
    expect(report.candidates[0]?.failures).toEqual([]);
    expect(report.candidates[0]?.noisy.map((n) => n.id)).toEqual(["ships/sql/real/wall-cold"]);
    expect(report.candidates[0]?.packs[0]?.status).toBe("noisy");
  });

  it("names a pack's worst confirmed failure, the phase that moved and the rules behind it", () => {
    const report = buildReport(
      verdict([
        pack(
          "typescript",
          [
            // A higher ratio that is only noise does not hide the confirmed failure.
            cell("fixtures", [measured("wall-warm", [100, 100, 100], [300, 300, 300], [[100, 100, 100], [105, 105, 105]])]),
            cell("real", [measured("wall-cold", [8000, 8080, 8130], [17600, 17690, 17900], [[8000, 8050, 8100], [17500, 17600, 17700]], "evaluate: load +0 ms, compile +10 ms, evaluate +9500 ms, startup +5 ms")]),
          ],
          {
            oneRule: [
              oneRule("fixtures"),
              oneRule("real", [rule("no-shadow", [3700, 3700], [12100, 12100], [[], []], "evaluate: compile +5 ms, evaluate +8395 ms"), rule("member-ordering", [520, 520], [690, 690], [[520, 520, 520], [560, 560, 560]])]),
            ],
          },
        ),
      ]),
      meta,
    );
    const p = report.candidates[0]!.packs[0]!;
    expect(report.verdict.status).toBe("FAIL");
    expect(p.summary.worst).toMatchObject({ id: "ships/typescript/real/wall-cold", status: "fail" });
    expect(p.summary.hint).toBe("`no-shadow` +8.40 s of +8.57 s");
    const failures = report.candidates[0]!.failures;
    expect(failures.map((f) => [f.id, f.slice])).toEqual([
      ["ships/typescript/real/wall-cold", "A"],
      ["ships/typescript/one-rule/real/no-shadow/time", "B"],
    ]);
    expect(failures.map((f) => f.locator)).toEqual(["evaluate: load +0 ms, compile +10 ms, evaluate +9500 ms, startup +5 ms", "evaluate: compile +5 ms, evaluate +8395 ms"]);
    expect(report.candidates[0]!.noisy.map((n) => n.id)).toEqual(["ships/typescript/fixtures/wall-warm", "ships/typescript/one-rule/real/member-ordering/time"]);
  });

  it("fails a pack with a rule the candidate cannot run, whatever its timings", () => {
    const report = buildReport(verdict([pack("blade", [cell("real", [measured("wall-cold", [700], [700])])], { missing: [{ rule: "blade-x", reason: "no .rerail counterpart" }] })]), meta);
    expect(report.candidates[0]?.status).toBe("FAIL");
    expect(report.candidates[0]?.failures.map((f) => f.item)).toEqual(["coverage"]);
  });

  it("labels a .rerail cell's chunked figures and says how the whole-pack run got on", () => {
    const rr: CellVerdict = {
      ...cell("real", [measured("wall-cold", [700], [900])]),
      completion: { status: "fail", kind: "candidate-failed", reason: "rr.cold1: killed at 2700 s", stoppedOn: "r9 (policy 9 of 20)" },
      caps: { memCap: ["56G"], timeCapS: [14400] },
      chunked: { size: 10, chunks: 2, rulesRun: 20, rulesOf: 20, failedChunks: [] },
      progress: { started: 9, of: 20, wallMs: 2_700_000, rssKB: 2 * 1048576 },
      grouping: { policies: 20, combined: 18, alone: 2, largest: 6 },
      fallback: [],
    };
    const v = verdict([pack("python", [rr], { oneRule: [], oneFile: [], slicesNote: "the .rerail rewrites measure slice A only" })]);
    const report = buildReport({ ...v, candidates: [{ candidate: "rr", description: "the .rerail rewrites", packs: v.candidates[0]!.packs }], gate: ["rr"], machine: { ...v.machine, memCap: ["24G", "56G"], timeCapS: [2700, 14400] } }, meta);
    const cellItem = report.candidates[0]!.packs[0]!.cells[0]!;
    expect(cellItem.chunked).toMatchObject({ chunks: 2, rules_run: 20 });
    expect(report.candidates[0]!.failures.find((f) => f.item === "wall-cold")?.reason).toMatch(/chunked: the sum of 2 chunk runs/);
    const md = renderReport(report);
    expect(md).toContain("chunked 700 ms → 900 ms");
    expect(md).toContain("whole-pack run got to policy 9 of 20 in 2700 s, 2.0 GiB peak RSS");
    expect(md).toContain("ran combined: 18 of 20 policies in groups (largest 6), 2 alone");
    expect(md).toContain("caps 56G, 14400 s");
  });
});

describe("slice B in the report", () => {
  const report = () =>
    buildReport(
      verdict([
        pack("python", [cell("real", passing())], {
          oneRule: [
            oneRule("real", [
              rule("same", [100, 100], [105, 105]),
              // Over 1.15× at first and after confirmation, but by 6 ms: under the floor, not counted.
              rule("tiny", [4, 4], [12, 12], [[4, 4, 4], [10, 10, 10]]),
              // Over at first, within at confirmation: noisy.
              rule("noisy", [100, 100], [150, 150], [[100, 100, 100], [110, 110, 110]]),
              // Still over after confirmation by 90 ms: counted, and over 1.5×.
              rule("slow", [100, 100], [200, 200], [[100, 100, 100], [190, 190, 190]], "evaluate: compile +0 ms, evaluate +90 ms"),
            ]),
          ],
        }),
      ]),
      meta,
    );

  it("counts a rule over the bar only when still over after confirmation and moved by at least 20 ms", () => {
    const r = report();
    expect(r.verdict.status).toBe("FAIL");
    expect(r.candidates[0]!.failures.map((f) => [f.id, f.locator])).toEqual([["ships/python/one-rule/real/slow/time", "evaluate: compile +0 ms, evaluate +90 ms"]]);
    expect(r.candidates[0]!.noisy.map((f) => f.id)).toEqual(["ships/python/one-rule/real/noisy/time"]);
  });

  it("keeps every rule over the bar before the filter, the median and p95 ratio, and the worst five with ms", () => {
    const b = report().candidates[0]!.packs[0]!.one_rule[0]!;
    expect(b.stats).toMatchObject({ rules: 4, compared: 4, over_bar_before: ["tiny", "noisy", "slow"], failing: ["slow"], over_alert: ["slow"], p95_ratio: 3 });
    expect(b.stats.worst.map((w) => [w.rule, w.baseline_ms, w.candidate_ms])).toEqual([
      ["tiny", 4, 10],
      ["slow", 100, 190],
      ["noisy", 100, 110],
      ["same", 100, 105],
    ]);
    expect(b.rules.find((r) => r.rule === "tiny")?.time).toMatchObject({ time_status: "under_floor", status: "pass", over_before: true });
    const summary = report().candidates[0]!.packs[0]!.summary.slices.B;
    expect(summary).toMatchObject({ status: "fail", rules: 4, fixed_wall: { real: 1.0125 } });
    expect(summary.stats).toMatchObject({ failing: ["real/slow"], over_bar_before: ["real/tiny", "real/noisy", "real/slow"] });
  });

  it("fails a rule whose findings differ between the sides, and one that fails on the candidate only", () => {
    const differs: OneRuleTiming = { ...rule("differs", [50, 50], [50, 50]), findings: { both: 2, mainOnly: 1, candidateOnly: 0, samples: ["main only differs a.py 3 X"] } };
    const broken: OneRuleTiming = { ...rule("broken", [50, 50], [50, 50]), ruleFailure: "compile: bad" };
    const r = buildReport(verdict([pack("python", [cell("real", passing())], { oneRule: [oneRule("real", [differs, broken])] })]), meta);
    expect(r.candidates[0]!.failures.map((f) => [f.id, f.locator])).toEqual([
      ["ships/python/one-rule/real/broken/rule-failure", "compile"],
      ["ships/python/one-rule/real/differs/findings", "evaluate: findings differ"],
    ]);
    expect(r.candidates[0]!.packs[0]!.summary.slices.B.findings).toEqual({ identical: 1, differing: 1 });
  });

  it("fails a fixed cost (the empty rule set) over the bar, with its locator", () => {
    const b = oneRule("real");
    const fixed = { ...b.fixed!, times: [judgeTime("wall-cold", [800, 800], [1000, 1000], [800, 800, 800], [990, 990, 990]), ...b.fixed!.times.slice(1)], locator: "startup: load +0 ms, compile +0 ms, evaluate +0 ms, startup +190 ms" };
    const r = buildReport(verdict([pack("python", [cell("real", passing())], { oneRule: [{ ...b, fixed }] })]), meta);
    expect(r.candidates[0]!.failures.map((f) => [f.id, f.locator])).toEqual([["ships/python/one-rule/real/fixed/wall-cold", "startup: load +0 ms, compile +0 ms, evaluate +0 ms, startup +190 ms"]]);
  });
});

describe("slice C in the report", () => {
  it("judges each file's metrics and findings, names the file and the phase that moved", () => {
    const slow = oneFile("largest", [measured("wall-cold", [400, 400, 400], [600, 600, 600], [[400, 400, 400], [590, 590, 590]], "evaluate: load +0 ms, compile +0 ms, evaluate +190 ms, startup +0 ms"), measured("rss-cold", [100_000], [100_000])]);
    const differs = oneFile("small", undefined, { findings: { both: 1, mainOnly: 0, candidateOnly: 2, samples: ["candidate only r src/small.ts 1 X"] } });
    const r = buildReport(verdict([pack("react", [cell("real", passing())], { oneFile: [slow, oneFile("median"), differs] })]), meta);
    expect(r.verdict.status).toBe("FAIL");
    expect(r.candidates[0]!.failures.map((f) => [f.id, f.slice, f.locator])).toEqual([
      ["ships/react/one-file/largest/wall-cold", "C", "evaluate: load +0 ms, compile +0 ms, evaluate +190 ms, startup +0 ms"],
      ["ships/react/one-file/small/findings", "C", "evaluate: findings differ"],
    ]);
    expect(r.candidates[0]!.failures[0]!.reason).toContain("largest: src/largest.ts");
    const c = r.candidates[0]!.packs[0]!.summary.slices.C;
    expect(c.status).toBe("fail");
    expect(c.files.map((f) => [f.label, f.worst_ratio, f.metrics_status, f.findings_same])).toEqual([
      ["largest", 1.5, "fail", true],
      ["median", 1, "pass", true],
      ["small", 1, "pass", false],
    ]);
    const md = renderReport(r);
    expect(md).toContain("largest: `src/largest.ts` (2.0 KB)");
    expect(md).toContain("**0 baseline only, 2 candidate only**");
  });

  it("passes a noisy file metric, reported apart", () => {
    const noisy = oneFile("median", [measured("cpu-warm", [400, 400, 400], [500, 500, 500], [[400, 400, 400], [410, 410, 410]])]);
    const r = buildReport(verdict([pack("react", [cell("real", passing())], { oneFile: [oneFile("largest"), noisy, oneFile("small")] })]), meta);
    expect(r.verdict.status).toBe("PASS");
    expect(r.candidates[0]!.noisy.map((n) => [n.id, n.slice])).toEqual([["ships/react/one-file/median/cpu-warm", "C"]]);
  });
});

describe("a missing slice", () => {
  const cells = (): CellVerdict[] => [cell("fixtures", passing()), cell("real", passing())];

  it("makes a pack without slice B on a corpus INCOMPLETE, never PASS", () => {
    const r = buildReport(verdict([pack("node", cells(), { oneRule: [oneRule("real")] })]), meta);
    expect(r.verdict.status).toBe("INCOMPLETE");
    const p = r.candidates[0]!.packs[0]!;
    expect(p.status).toBe("incomplete");
    expect(p.incomplete).toEqual([{ slice: "B", reason: "fixtures: not measured" }]);
    expect(p.summary.slices.B.status).toBe("missing");
    expect(renderReport(r)).toContain("- slice B: fixtures: not measured");
  });

  it("makes a pack with fewer than three slice C files INCOMPLETE", () => {
    const r = buildReport(verdict([pack("node", cells(), { oneFile: [oneFile("largest"), oneFile("median")] })]), meta);
    expect(r.verdict.status).toBe("INCOMPLETE");
    expect(r.candidates[0]!.packs[0]!.incomplete).toEqual([{ slice: "C", reason: "2 of 3 files measured" }]);
  });

  it("makes a pack whose slice B left rules untimed INCOMPLETE", () => {
    const r = buildReport(verdict([pack("node", cells(), { oneRule: [oneRule("fixtures"), { ...oneRule("real"), unmeasured: ["late-rule"] }] })]), meta);
    expect(r.candidates[0]!.packs[0]!.incomplete).toEqual([{ slice: "B", reason: "real: 1 rule not timed on both sides (late-rule)" }]);
  });

  it("does not count a corpus the pack has none of", () => {
    const absent: CellVerdict = { ...cell("fixtures", []), note: "no fixtures corpus: the pack has no upstream recording (no conformance.json)" };
    const r = buildReport(verdict([pack("rerail", [absent, cell("real", passing())], { oneRule: [{ ...oneRule("fixtures"), note: "no fixtures corpus: the pack has no upstream recording (no conformance.json)" }, oneRule("real")] })]), meta);
    expect(r.verdict.status).toBe("PASS");
  });

  it("makes a candidate that measures slice A only INCOMPLETE", () => {
    const v = verdict([pack("sql", cells(), { oneRule: [], oneFile: [], slicesNote: "the .rerail rewrites measure slice A only" })]);
    const r = buildReport({ ...v, candidates: [{ candidate: "rr", description: "the .rerail rewrites", packs: v.candidates[0]!.packs }], gate: ["rr"] }, meta);
    expect(r.verdict.status).toBe("INCOMPLETE");
    expect(r.candidates[0]!.packs[0]!.incomplete.map((m) => m.slice)).toEqual(["B", "C"]);
  });

  it("still FAILs a pack that fails and lacks a slice", () => {
    const r = buildReport(verdict([pack("node", [cell("fixtures", [measured("wall-cold", [100], [200])]), cell("real", passing())], { oneFile: [] })]), meta);
    expect(r.verdict.status).toBe("FAIL");
    expect(r.candidates[0]!.packs[0]!.status).toBe("fail");
    expect(r.candidates[0]!.packs[0]!.incomplete).toHaveLength(1);
  });
});

describe("a partial run", () => {
  const packs = (): PackVerdict[] => [pack("python", [cell("real", passing())])];

  it("is PARTIAL, not PASS, when narrowed and everything measured passes", () => {
    const r = buildReport(verdict(packs(), ["--packs python: 1 of 13 packs"]), meta);
    expect(r.verdict).toMatchObject({ status: "PARTIAL", partial: ["--packs python: 1 of 13 packs"] });
    expect(r.candidates[0]!.status).toBe("PARTIAL");
    expect(renderReport(r)).toContain("**Partial run**: --packs python: 1 of 13 packs. A partial verdict is never PASS.");
  });

  it("still FAILs when narrowed and something fails", () => {
    const r = buildReport(verdict([pack("python", [cell("real", [measured("wall-cold", [100], [200])])])], ["--corpus real: slice A and B on real only"]), meta);
    expect(r.verdict.status).toBe("FAIL");
  });

  it("is PASS only when every pack is measured in all three slices and nothing narrowed it", () => {
    expect(buildReport(verdict(packs()), meta).verdict.status).toBe("PASS");
  });
});
