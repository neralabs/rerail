import { describe, expect, it } from "vitest";
import {
  bannerFor, firstOpenState, liveState, overviewBody, rulesOnSource, steeredTally, topRules, LIVE_MINUTES, TOP_RULES,
  type RepositoryFacts,
} from "../../ui/first-open.js";

const MINUTE = 60_000;

const activity = (distinctViolations: number, loggedViolations: number, rerails = 0, measured = true) =>
  ({ measured, distinctViolations, loggedViolations, rerails, sessions: 0, lastSeenAt: null, audits: 1 });

describe("Violations steered tile", () => {
  it("counts violations recorded in Steer or Block mode, of every violation in range", () => {
    const tally = steeredTally({ a: activity(5, 2), b: activity(3, 3), c: activity(4, 0) });

    expect(tally).toEqual({ steered: 7, total: 12 });
  });

  it("ignores steer deliveries, which can outnumber the violations", () => {
    // One violation steered three times (the agent kept repeating it) is one steered violation.
    const tally = steeredTally({ a: activity(1, 0, 3) });

    expect(tally).toEqual({ steered: 1, total: 1 });
    expect(tally.steered).toBeLessThanOrEqual(tally.total);
  });

  it("leaves unmeasured rules out and reads zero for an empty range", () => {
    expect(steeredTally({ convo: activity(2, 0, 0, false) })).toEqual({ steered: 0, total: 0 });
    expect(steeredTally({})).toEqual({ steered: 0, total: 0 });
  });

  it("keeps a rule switched to Log after steering counted as steered for what it steered", () => {
    // Two violations recorded while on Steer, one after the switch to Log.
    expect(steeredTally({ r: activity(3, 1) })).toEqual({ steered: 2, total: 3 });
  });
});

describe("Most violations logged", () => {
  const rule = (id: string, title: string, fires: number) => ({ id, title, fires });

  it("lists only rules with violations, most first, at most ten", () => {
    const rules = Array.from({ length: 14 }, (_, i) => rule(`r${i}`, `Rule ${String(i).padStart(2, "0")}`, i));

    const top = topRules(rules);

    expect(top).toHaveLength(TOP_RULES);
    expect(top.map(r => r.fires)).toEqual([13, 12, 11, 10, 9, 8, 7, 6, 5, 4]);
    expect(topRules([rule("a", "A", 0)])).toEqual([]);
  });

  it("breaks ties by title, then id, whatever order the rules arrive in", () => {
    const rules = [rule("z", "Beta", 3), rule("b", "Alpha", 3), rule("a", "Alpha", 3), rule("c", "Gamma", 5)];

    expect(topRules(rules).map(r => r.id)).toEqual(["c", "a", "b", "z"]);
    expect(topRules([...rules].reverse()).map(r => r.id)).toEqual(["c", "a", "b", "z"]);
  });

  it("keeps the tenth place for the first of tied rules at the cut", () => {
    const rules = [...Array.from({ length: 9 }, (_, i) => rule(`top${i}`, `Top ${i}`, 10)), rule("y", "Y", 1), rule("x", "X", 1)];

    expect(topRules(rules).at(-1)?.id).toBe("x");
  });

  it("does not reorder the caller's list", () => {
    const rules = [rule("a", "A", 1), rule("b", "B", 2)];
    topRules(rules);
    expect(rules.map(r => r.id)).toEqual(["a", "b"]);
  });
});

describe("first-open state", () => {
  const base: RepositoryFacts = { scoped: true, missing: false, setUp: true, stopped: false, ruleCount: 12, sessionsEver: 3 };

  it.each<[string, Partial<RepositoryFacts>, string]>([
    ["no repository in the URL", { scoped: false, missing: true, setUp: false }, "no-repo"],
    ["a repository the daemon does not know", { missing: true, setUp: false }, "missing"],
    ["never set up", { setUp: false, ruleCount: 0, sessionsEver: 0 }, "not-set-up"],
    ["stopped, whatever it holds", { stopped: true, ruleCount: 0, sessionsEver: 0 }, "stopped"],
    ["set up with nothing added", { ruleCount: 0, sessionsEver: 4 }, "no-packs"],
    ["rules but no session yet", { sessionsEver: 0 }, "packs-no-session"],
    ["rules and sessions", {}, "mature"],
  ])("%s", (_, facts, state) => {
    expect(firstOpenState({ ...base, ...facts })).toBe(state);
  });

  it("shows no banner without rules (decision 2), the welcome until dismissed, Start when stopped", () => {
    expect(bannerFor("no-packs", false)).toBeNull();
    expect(bannerFor("packs-no-session", false)).toBe("welcome");
    expect(bannerFor("packs-no-session", true)).toBeNull();
    expect(bannerFor("stopped", true)).toBe("stopped");
    expect(bannerFor("mature", false)).toBeNull();
  });

  it("moves from the welcome to no banner once the first session is recorded", () => {
    const before = firstOpenState({ ...base, sessionsEver: 0 });
    const after = firstOpenState({ ...base, sessionsEver: 1 });

    expect([bannerFor(before, false), bannerFor(after, false)]).toEqual(["welcome", null]);
  });

  it.each<[{ ruleCount: number; sessionsEver: number; topCount: number }, string]>([
    [{ ruleCount: 0, sessionsEver: 5, topCount: 0 }, "no-rules"],
    [{ ruleCount: 4, sessionsEver: 0, topCount: 0 }, "no-sessions"],
    [{ ruleCount: 4, sessionsEver: 2, topCount: 0 }, "no-violations"],
    [{ ruleCount: 4, sessionsEver: 2, topCount: 1 }, "lists"],
  ])("body for %o is %s", (facts, body) => {
    expect(overviewBody(facts)).toBe(body);
  });
});

describe("Rules on tile line", () => {
  it("names the packs, adds yours, and says when there is nothing", () => {
    expect(rulesOnSource(1, 0)).toBe("in 1 rule pack");
    expect(rulesOnSource(3, 2)).toBe("in 3 rule packs and yours");
    expect(rulesOnSource(0, 2)).toBe("all created by you");
    expect(rulesOnSource(0, 0)).toBe("no rules added yet");
  });
});

describe("a recent session's live state", () => {
  const now = 1_000 * MINUTE;

  it("is live up to the limit since the newest event, then stale", () => {
    const open = { endedAt: null, startedAt: now - 120 * MINUTE };

    expect(liveState({ ...open, lastEventAt: now - LIVE_MINUTES * MINUTE }, now)).toBe("live");
    expect(liveState({ ...open, lastEventAt: now - LIVE_MINUTES * MINUTE - 1 }, now)).toBe("stale");
  });

  it("measures from the start when no event was recorded, and is none once ended", () => {
    expect(liveState({ endedAt: null, startedAt: now - 5 * MINUTE, lastEventAt: null }, now)).toBe("live");
    expect(liveState({ endedAt: now - MINUTE, startedAt: now - 5 * MINUTE, lastEventAt: now - MINUTE }, now)).toBeNull();
  });
});
