import { describe, expect, it } from "vitest";
import { dimConflict, type ConflictSignals, type DimmerBudget, type DimmerPrevious } from "../../../src/engine/index.js";

const ROOMY: DimmerBudget = { stops: 1, checkpoints: 3 };

/**
 * Signals with no blast radius, even odds and a day to merge, so the score is `12 × growthPerHour × lockIn`: against
 * the fixed thresholds (interruptCost 300) ambient from 75, checkpoint from 300, stop from 1,200.
 */
const signals = (over: Partial<ConflictSignals> = {}): ConflictSignals => ({
  quiet: false, lockIn: 1, lockInClass: "local", blastRadius: 0, callers: 0, otherRefs: 0, growthPerHour: 10,
  linesTheirs: 10, linesMine: 0, windowHours: 1, pLose: 0.5, hoursToMerge: 24, pullRequest: false,
  ...over,
});

const level = (over: Partial<ConflictSignals>, previous: DimmerPrevious | null = null, budget: DimmerBudget = ROOMY) =>
  dimConflict({ signals: signals(over), previous, budget });

describe("the dimmer", () => {
  it("scores expected rework lines: P(lose) × growth × time to merge × lock-in × (1 + log2(1 + blast radius))", () => {
    expect(level({ growthPerHour: 10 }).score).toBe(120);
    expect(level({ growthPerHour: 10, lockIn: 2, blastRadius: 3 }).score).toBe(0.5 * 10 * 24 * 2 * 3);
    expect(level({ growthPerHour: 10, pLose: 1, hoursToMerge: 48 }).score).toBe(480);
  });

  it("picks the level from the score against the cost of an interruption", () => {
    expect(level({ growthPerHour: 6 }).level).toBe("logged");
    expect(level({ growthPerHour: 10 }).level).toBe("ambient");
    expect(level({ growthPerHour: 25 }).level).toBe("checkpoint");
    expect(level({ growthPerHour: 60, lockIn: 4, lockInClass: "persisted" }).level).toBe("stop");
  });

  it("logs quiet files whatever the score", () => {
    expect(level({ quiet: true, growthPerHour: 500, lockIn: 4 })).toMatchObject({ level: "logged", reasons: ["only lockfiles, changelogs, generated output or docs"] });
  });

  it("stops the line only for lock-in ×4, fast growth and four interruptions' worth of rework", () => {
    const stop = { lockIn: 4 as const, lockInClass: "persisted" as const };
    expect(level({ ...stop, growthPerHour: 60 }).level).toBe("stop");
    // Each condition missing leaves a checkpoint.
    expect(level({ ...stop, growthPerHour: 50 }).level).toBe("checkpoint");
    expect(level({ lockIn: 2, lockInClass: "exported", growthPerHour: 120 }).level).toBe("checkpoint");
    expect(level({ ...stop, growthPerHour: 60, pLose: 0.1 }).level).toBe("checkpoint");
  });

  it("never drops an open conflict's level, and raises it only past the next threshold by 20%", () => {
    expect(level({ growthPerHour: 0 }, { level: "checkpoint", queuedFor: null }).level).toBe("checkpoint");
    // 320 clears 300 but not 360: a conflict already ambient stays there; one seen for the first time is a checkpoint.
    expect(level({ growthPerHour: 320 / 12 }, { level: "ambient", queuedFor: null }).level).toBe("ambient");
    expect(level({ growthPerHour: 320 / 12 }).level).toBe("checkpoint");
    expect(level({ growthPerHour: 30 }, { level: "ambient", queuedFor: null }).level).toBe("checkpoint");
    expect(level({ growthPerHour: 80 / 12 }, { level: "logged", queuedFor: null }).level).toBe("logged");
    expect(level({ growthPerHour: 90 / 12 }, { level: "logged", queuedFor: null }).level).toBe("ambient");
  });

  it("queues a checkpoint or a stop over budget for that level, and raises it once the budget frees, whatever its score by then", () => {
    expect(level({ growthPerHour: 25 }, null, { stops: 1, checkpoints: 0 })).toMatchObject({ level: "ambient", queuedFor: "checkpoint" });
    // A stop the session has no stop left for is a checkpoint while one fits today, else ambient; queued for the stop either way.
    const stopWorthy = { growthPerHour: 60, lockIn: 4 as const, lockInClass: "persisted" as const };
    expect(level(stopWorthy, null, { stops: 0, checkpoints: 3 })).toMatchObject({ level: "checkpoint", queuedFor: "stop" });
    expect(level(stopWorthy, null, { stops: 0, checkpoints: 0 })).toMatchObject({ level: "ambient", queuedFor: "stop" });
    // A checkpoint already shown keeps its level while the stop it calls for waits.
    expect(level(stopWorthy, { level: "checkpoint", queuedFor: null }, { stops: 0, checkpoints: 0 })).toMatchObject({ level: "checkpoint", queuedFor: "stop" });
    // Queued, it keeps waiting for its level though the score fell, and rises to it once the budget frees.
    expect(level({ growthPerHour: 1 }, { level: "ambient", queuedFor: "stop" }, { stops: 0, checkpoints: 0 })).toMatchObject({ level: "ambient", queuedFor: "stop" });
    expect(level({ growthPerHour: 1 }, { level: "checkpoint", queuedFor: "stop" }, { stops: 1, checkpoints: 0 })).toMatchObject({ level: "stop", queuedFor: null });
    expect(level({ growthPerHour: 1 }, { level: "ambient", queuedFor: "checkpoint" })).toMatchObject({ level: "checkpoint", queuedFor: null });
  });

  it("gives a plain sentence per signal", () => {
    expect(level({
      lockIn: 2, lockInClass: "exported", callers: 14, otherRefs: 2, blastRadius: 16, linesTheirs: 380, linesMine: 12,
      windowHours: 14, growthPerHour: 28, pullRequest: true,
    }).reasons).toEqual([
      "exported signature: ×2",
      "+380 lines on theirs and +12 on this branch in 14 h",
      "14 callers, and 2 other branches touch the same files",
      "pull request open: about 24 h to merge",
      "even odds either side loses",
    ]);
    expect(level({ callers: 1, linesTheirs: 3, hoursToMerge: 72.4 }).reasons)
      .toEqual(["local code: ×1", "+3 lines on theirs in 1 h", "1 caller", "about 72 h to merge", "even odds either side loses"]);
  });
});
