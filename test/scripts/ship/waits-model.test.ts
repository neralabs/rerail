// scripts/ship/waits-model.ts: how the user waits are judged. A latency distribution fails only when the bootstrap 95%
// CI of its p50 or p95 ratio lies wholly above 1.10; a rate fails when the candidate's is higher; a single wait fails on
// its median, and counts only when its confirmation runs fail too; a set absolute limit fails the candidate whatever the
// baseline does; a scenario whose candidate runs fail where the baseline's complete fails.

import { describe, expect, it } from "vitest";
import { bootstrapRatio, buildWaitsSection, compare, failingScenarios, quantile, SCENARIOS, type WaitRun, type WaitSide, waitItem } from "../../../scripts/ship/waits-model.js";

function run(wait: WaitRun["wait"], scenario: string, side: WaitSide, samples: Record<string, number[]>, extra: Partial<WaitRun> = {}): WaitRun {
  return {
    wait, scenario, side, phase: "runs", rep: 1, ok: true, reason: null, samples, notes: {},
    load: 0, pinnedBusy: 0, siblingBusy: 0, cacheDropped: null, started: "2026-11-16T00:00:00Z", host: "h", cpus: "8-11", ...extra,
  };
}

const config = { limits: {} as Record<string, number | null>, reportOnly: ["W4"], reproduce: () => "npm run bench:ship -- waits" };
const spec = (id: string) => SCENARIOS.find((s) => `wait/${s.wait}/${s.scenario}` === id)!;
/** n values spread evenly around `center` (± 10%), in a fixed order. */
const around = (center: number, n: number): number[] => Array.from({ length: n }, (_, i) => center * (0.9 + (0.2 * ((i * 37) % n)) / n));

describe("quantiles and their ratio", () => {
  it("interpolates between order statistics", () => {
    expect(quantile([1, 2, 3, 4], 0.5)).toBe(2.5);
    expect(quantile([10, 0, 20], 0.95)).toBeCloseTo(19);
  });

  it("puts the true ratio inside a narrow interval when samples are plentiful, and is reproducible", () => {
    const baseline = around(100, 400);
    const candidate = around(120, 400);
    const first = bootstrapRatio(baseline, candidate, 0.5, "seed");
    expect(first.ratio).toBeCloseTo(1.2, 1);
    expect(first.lo).toBeGreaterThan(1.1);
    expect(first.hi).toBeLessThan(1.3);
    expect(bootstrapRatio(baseline, candidate, 0.5, "seed")).toEqual(first);
  });
});

describe("distribution metrics", () => {
  const p95 = spec("wait/W1/replay").metrics.find((m) => m.metric === "p95")!;

  it("fails only when the whole interval lies above the bar", () => {
    expect(compare(p95, around(100, 400), around(130, 400), "x").status).toBe("fail");
    // 1.12x at the point, but with a handful of events the interval holds the bar: not a failure, flagged as close.
    const few = compare(p95, around(100, 6), around(112, 6), "x");
    expect(few.ratio).toBeGreaterThan(1.1);
    expect(few.status).toBe("pass");
    expect(few.within_spread).toBe(true);
  });

  it("fails a late-answer rate only when the candidate's is higher", () => {
    const late = spec("wait/W1/replay").metrics.find((m) => m.metric === "late_rate")!;
    expect(compare(late, [0, 0, 1, 0], [0, 1, 0, 0], "x").status).toBe("pass");
    expect(compare(late, [0, 0, 0, 0], [0, 1, 0, 0], "x").status).toBe("fail");
  });
});

describe("single waits", () => {
  const scan = spec("wait/W5/scan-warm");

  it("counts a failing median only when its confirmation runs fail too", () => {
    const first = [run("W5", "scan-warm", "baseline", { wall_ms: [700] }), run("W5", "scan-warm", "candidate", { wall_ms: [800] })];
    const confirmPasses = [...first, run("W5", "scan-warm", "baseline", { wall_ms: [700] }, { phase: "confirm" }), run("W5", "scan-warm", "candidate", { wall_ms: [720] }, { phase: "confirm" })];
    expect(waitItem(scan, first, config).status).toBe("fail");
    expect(waitItem(scan, confirmPasses, config).status).toBe("noisy");
    expect(failingScenarios(first, config).map((s) => s.scenario)).toEqual(["scan-warm"]);
  });

  it("fails a set limit even when the baseline is as slow, and reports an unset one", () => {
    const runs = [run("W5", "scan-warm", "baseline", { wall_ms: [12_000] }), run("W5", "scan-warm", "candidate", { wall_ms: [12_100] })];
    expect(waitItem(scan, runs, config).status).toBe("pass");
    expect(waitItem(scan, runs, config).metrics[0]!.limit).toMatchObject({ name: "W5.scan_wait_ms", value: null, candidate_within: null });
    const limited = waitItem(scan, runs, { ...config, limits: { "W5.scan_wait_ms": 10_000 } });
    expect(limited.status).toBe("fail");
    expect(limited.hint).toContain("over the limit");
  });

  it("fails a scenario whose candidate runs fail where the baseline's complete", () => {
    const runs = [run("W5", "scan-warm", "baseline", { wall_ms: [700] }), run("W5", "scan-warm", "candidate", {}, { ok: false, reason: "rerail scan exited 1" })];
    const item = waitItem(scan, runs, config);
    expect(item.status).toBe("fail");
    expect(item.reason).toContain("rerail scan exited 1");
  });
});

describe("the section", () => {
  it("fails on a counted scenario, not on a report-only one, and lists what was not run", () => {
    const runs = [
      run("W4", "upgrade", "baseline", { first_verdict_ms: [1000] }), run("W4", "upgrade", "candidate", { first_verdict_ms: [2000] }),
      run("W7", "status", "baseline", { wall_ms: [200] }), run("W7", "status", "candidate", { wall_ms: [205] }),
    ];
    const section = buildWaitsSection(runs, {
      ...config, inputs: { sessions: {}, repos: {} }, method: [], skipped: {},
      requested: ["wait/W4/upgrade", "wait/W7/status", "wait/W7/packs-list"],
    });
    expect(section.items.find((i) => i.id === "wait/W4/upgrade")).toMatchObject({ status: "fail", gated: false });
    expect(section.status).toBe("INCOMPLETE");
    expect(section.not_measured.map((n) => n.id)).toEqual(["wait/W7/packs-list"]);
  });
});
