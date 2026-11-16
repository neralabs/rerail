// scripts/ship/report-model.ts: what the ship / no-ship report counts. A metric failing at the median counts only when
// its confirmation runs fail too (else it is noisy, reported and not counted); a slowdown is attributed to the rule that
// carries most of it, or reported as spread over every rule; a pack's row names its worst failing metric and that
// metric's cause.

import { describe, expect, it } from "vitest";
import { LINUX_METHOD } from "../../../scripts/ship/platform.js";
import { attribute, buildReport, drillHint, metricStatus, type RunMeta } from "../../../scripts/ship/report-model.js";
import type { CellVerdict, MeasuredMetric, RuleTiming, Verdict } from "../../../scripts/ship/shipbar.js";
import { compareMetric } from "../../../scripts/ship/verdict.js";

function measured(metric: string, main: number[], candidate: number[], confirm?: [number[], number[]]): MeasuredMetric {
  return {
    verdict: compareMetric(metric, main, candidate),
    mainRuns: main,
    candidateRuns: candidate,
    confirm: confirm === undefined ? null : { verdict: compareMetric(metric, confirm[0], confirm[1]), mainRuns: confirm[0], candidateRuns: confirm[1] },
  };
}

describe("confirmation", () => {
  it("counts a failing metric only when its confirmation runs fail too, else reports it as noise", () => {
    expect(metricStatus(measured("wall-cold", [700, 710, 720], [800, 810, 820], [[700, 705, 710], [740, 745, 750]]))).toBe("noisy");
    expect(metricStatus(measured("wall-cold", [700, 710, 720], [800, 810, 820], [[700, 705, 710], [800, 805, 810]]))).toBe("fail");
  });

  it("keeps a failure that was never confirmed", () => {
    expect(metricStatus(measured("wall-cold", [700, 710, 720], [800, 810, 820]))).toBe("fail");
  });
});

describe("attributing a slowdown to rules", () => {
  const drill = (rules: RuleTiming[]) => {
    const { rules: items, totalAddedMs } = attribute(rules);
    return { measure: "evaluate_ms" as const, reps: 2, startup: { baseline_ms: 50, candidate_ms: 50 }, total_added_ms: totalAddedMs, rules: items };
  };

  it("names one rule alone when it carries at least half of the added time", () => {
    const d = drill([
      { rule: "no-shadow", main: 3700, candidate: 12100 },
      { rule: "member-ordering", main: 520, candidate: 690 },
      { rule: "no-redeclare", main: 210, candidate: 290 },
    ]);
    expect(d.rules[0]?.rule).toBe("no-shadow");
    expect(drillHint(d)).toBe("`no-shadow` +8.40 s of +8.65 s");
  });

  it("reports a regression spread over every rule, not one culprit", () => {
    const d = drill(["a", "b", "c", "d"].map((rule) => ({ rule, main: 100, candidate: 120 })));
    expect(drillHint(d)).toMatch(/^spread over 4 rules: \+20 ms each \(median\)/);
  });

  it("puts a rule one side could not run last, with no added time", () => {
    const d = drill([
      { rule: "broken", main: 100, candidate: null },
      { rule: "slower", main: 100, candidate: 300 },
    ]);
    expect(d.rules.map((r) => [r.rule, r.added_ms])).toEqual([
      ["slower", 200],
      ["broken", null],
    ]);
  });
});

describe("the report", () => {
  const cell = (corpus: "fixtures" | "real", metrics: MeasuredMetric[], drill: CellVerdict["drill"] = null): CellVerdict => ({
    corpus,
    note: "",
    blocked: null,
    completion: { status: "pass", kind: "both-completed", reason: "", stoppedOn: null },
    ruleFailures: [],
    metrics,
    findings: null,
    agreement: { both: 10, mainOnly: 0, candidateOnly: 0, samples: [] },
    load: { main: [0.1], candidate: [0.1] },
    siblingBusyMax: 0,
    methods: [LINUX_METHOD],
    runs: { main: 6, candidate: 6 },
    rules: { main: 3, candidate: 3 },
    drill,
  });
  const verdict = (packs: Verdict["candidates"][number]["packs"]): Verdict => ({
    tolerancePercent: 10,
    runs: 3,
    baseline: { dir: "/b", sha: "b".repeat(40), ref: "v0.1.4" },
    candidate: { dir: "/c", sha: "c".repeat(40) },
    corpora: { baseline: "b".repeat(40), fixtures: {}, real: {} },
    machine: { hosts: ["h"], cpus: ["8,9,10,11"], threads: [4], memCap: ["24G"], timeCapS: [2700], methods: [{ method: LINUX_METHOD, runs: 12 }] },
    machines: [],
    candidates: [{ candidate: "ships", description: "as it ships", packs }],
    gate: ["ships"],
  });
  const meta: RunMeta = { startedAt: null, finishedAt: "2026-11-17T00:00:00Z", invokedBy: null, command: null, harnessSha: "h", confirmRuns: 3, drillReps: 2 };

  it("passes a candidate whose only failure is noise, and lists the noise apart", () => {
    const report = buildReport(verdict([{ pack: "sql", noBaseline: false, missing: [], extra: [], cells: [cell("real", [measured("wall-cold", [700, 710, 720], [800, 810, 820], [[700, 705, 710], [740, 745, 750]])])] }]), meta);
    expect(report.verdict.status).toBe("PASS");
    expect(report.candidates[0]?.failures).toEqual([]);
    expect(report.candidates[0]?.noisy.map((n) => n.id)).toEqual(["ships/sql/real/wall-cold"]);
    expect(report.candidates[0]?.packs[0]?.status).toBe("noisy");
  });

  it("names a pack's worst confirmed failure and the rule behind it", () => {
    const rules: RuleTiming[] = [
      { rule: "no-shadow", main: 3700, candidate: 12100 },
      { rule: "member-ordering", main: 520, candidate: 690 },
    ];
    const report = buildReport(
      verdict([
        {
          pack: "typescript",
          noBaseline: false,
          missing: [],
          extra: [],
          cells: [
            // A higher ratio that is only noise does not hide the confirmed failure.
            cell("fixtures", [measured("wall-warm", [100, 100, 100], [300, 300, 300], [[100, 100, 100], [105, 105, 105]])]),
            cell("real", [measured("wall-cold", [8000, 8080, 8130], [17600, 17690, 17900], [[8000, 8050, 8100], [17500, 17600, 17700]])], { measure: "evaluate_ms", reps: 2, startup: { main: 50, candidate: 50 }, rules }),
          ],
        },
      ]),
      meta,
    );
    const pack = report.candidates[0]!.packs[0]!;
    expect(report.verdict.status).toBe("FAIL");
    expect(pack.summary.worst).toMatchObject({ id: "ships/typescript/real/wall-cold", status: "fail" });
    expect(pack.summary.hint).toBe("`no-shadow` +8.40 s of +8.57 s");
    expect(report.candidates[0]!.failures.map((f) => f.id)).toEqual(["ships/typescript/real/wall-cold"]);
  });

  it("fails a pack with a rule the candidate cannot run, whatever its timings", () => {
    const report = buildReport(verdict([{ pack: "blade", noBaseline: false, missing: [{ rule: "blade-x", reason: "no .rr counterpart" }], extra: [], cells: [cell("real", [measured("wall-cold", [700], [700])])] }]), meta);
    expect(report.candidates[0]?.status).toBe("FAIL");
    expect(report.candidates[0]?.failures.map((f) => f.item)).toEqual(["coverage"]);
  });
});
