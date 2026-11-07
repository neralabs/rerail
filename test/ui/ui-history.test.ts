import { afterEach, describe, expect, it } from "vitest";
import { data, ui } from "../../ui/state.js";
import { filterRanked, historyEmpty, historyRuleId, rankedRules } from "../../ui/history-model.js";
import { parseRoute } from "../../ui/routes.js";

const NONE = { measured: true, distinctViolations: 0, sessions: 0, lastSeenAt: null, audits: 0, loggedViolations: 0, rerails: 0 };
const fired = (distinctViolations: number, extra: object = {}) => ({ ...NONE, distinctViolations, audits: 1, ...extra });

const storedRule = (id: string, title: string, enabled = true) => ({
  kind: "file",
  origin: "user",
  source: { kind: "library_rule", id, packId: null },
  folders: [""],
  enabled,
  rule: { id, mode: "steer", clauses: [{ violation: { message: title } }] },
});

afterEach(() => {
  data.rules = new Map();
  data.activity = {};
  data.activityAll = {};
  data.activitySessions = null;
  data.activityAllSessions = null;
  ui.actRule = null;
  ui.range = 7;
});

describe("History's rule list", () => {
  it("lists only rules with violations in the range, most distinct violations first, ties by title", () => {
    data.rules = new Map([["a", storedRule("a", "Beta")], ["b", storedRule("b", "Alpha")], ["c", storedRule("c", "Gamma")], ["d", storedRule("d", "Delta")]]);
    data.activity = { a: fired(2), b: fired(2), c: fired(5), d: fired(0) };

    expect(rankedRules().map(r => [r.id, r.count])).toEqual([["c", 5], ["b", 2], ["a", 2]]);
  });

  it("leaves out a rule that is not measured, and keeps one no longer here under its id", () => {
    data.rules = new Map([["a", storedRule("a", "Alpha")]]);
    data.activity = { a: fired(3, { measured: false }), gone: fired(1) };

    expect(rankedRules()).toEqual([{ id: "gone", title: "gone", origin: "No longer in this repo", count: 1 }]);
  });

  it("follows the range: the counts are the range's activity answer", () => {
    data.rules = new Map([["a", storedRule("a", "Alpha")], ["b", storedRule("b", "Beta")]]);
    data.activity = { a: fired(1), b: fired(4) };
    expect(rankedRules().map(r => r.id)).toEqual(["b", "a"]);

    data.activity = { a: fired(1) };
    expect(rankedRules().map(r => r.id)).toEqual(["a"]);
  });
});

describe("History's rule search", () => {
  it("matches title, id or where the rule comes from, in any case, keeping the ranking", () => {
    data.rules = new Map([["lib:cents", storedRule("lib:cents", "Store money as cents")], ["lib:fetch", storedRule("lib:fetch", "No raw fetch")]]);
    data.activity = { "lib:cents": fired(1), "lib:fetch": fired(4), gone: fired(2) };
    const ranked = rankedRules();

    expect(filterRanked(ranked, "  MONEY ").map(r => r.id)).toEqual(["lib:cents"]);
    expect(filterRanked(ranked, "lib:").map(r => r.id)).toEqual(["lib:fetch", "lib:cents"]);
    expect(filterRanked(ranked, "created by you").map(r => r.id)).toEqual(["lib:fetch", "lib:cents"]);
    expect(filterRanked(ranked, "")).toEqual(ranked);
    expect(filterRanked(ranked, "nothing like it")).toEqual([]);
  });
});

describe("the rule History shows", () => {
  it("is the top-ranked rule before one is chosen, and none when nothing is listed", () => {
    data.activity = { a: fired(1), b: fired(3) };
    expect(historyRuleId()).toBe("b");

    data.activity = {};
    expect(historyRuleId()).toBeNull();
  });

  it("is the linked rule, even one with no violations in the range, from #history?rule= and legacy #activity?rule=", () => {
    data.activity = { a: fired(3) };
    for (const hash of ["#history?rule=lib%3Aquiet", "#activity?rule=lib:quiet"]) {
      ui.actRule = parseRoute(hash).filterRule ?? null;
      expect(historyRuleId()).toBe("lib:quiet");
    }
  });
});

describe("what History says with no rule to list", () => {
  it("says to add rules while none is on, whatever ran", () => {
    data.rules = new Map([["a", storedRule("a", "Alpha", false)]]);
    data.activityAllSessions = { started: 4, withViolations: 0 };

    expect(historyEmpty()).toEqual({ kind: "noRules" });
  });

  it("waits for a session while rules are on and none ran since install", () => {
    data.rules = new Map([["a", storedRule("a", "Alpha")]]);
    data.activityAllSessions = { started: 0, withViolations: 0 };

    expect(historyEmpty()).toEqual({ kind: "noSessions" });
  });

  it("counts the audited sessions since install when none logged a violation", () => {
    data.rules = new Map([["a", storedRule("a", "Alpha")]]);
    data.activitySessions = { started: 1, withViolations: 0 };
    data.activityAllSessions = { started: 6, withViolations: 0 };
    data.activityAll = { a: { ...NONE, audits: 6 } };

    expect(historyEmpty()).toEqual({ kind: "noViolations", sessions: 6 });
  });

  it("reads since-install from the range's own answer when the range is Since install", () => {
    ui.range = 9999;
    data.rules = new Map([["a", storedRule("a", "Alpha")]]);
    data.activitySessions = { started: 2, withViolations: 0 };
    data.activityAllSessions = null;

    expect(historyEmpty()).toEqual({ kind: "noViolations", sessions: 2 });
  });

  it("is nothing in this range when violations exist only before it", () => {
    data.rules = new Map([["a", storedRule("a", "Alpha")]]);
    data.activityAllSessions = { started: 6, withViolations: 2 };
    data.activityAll = { a: fired(3) };

    expect(historyEmpty()).toEqual({ kind: "nothingInRange" });
  });

  it("is null once a rule is listed", () => {
    data.activity = { a: fired(1) };

    expect(historyEmpty()).toBeNull();
  });
});
