import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { activityView, data, isAudited, pagedList, summary, ui, type Violation } from "../../ui/state.js";
import { showMoreActivity } from "../../ui/poll.js";
import { violationOutcome } from "../../ui/derive.js";

const NONE = { measured: true, distinctViolations: 0, sessions: 0, lastSeenAt: null, audits: 0, loggedViolations: 0, rerails: 0 };

const storedRule = (id: string, mode: string, enabled: boolean) => ({
  kind: "file",
  origin: "user",
  source: { kind: "library_rule", id, packId: null },
  folders: [""],
  enabled,
  rule: { id, mode, clauses: [] },
});

const finding = (n: number, ruleId: string, mode = "steer"): Violation => ({ id: `v${n}`, auditId: `a${n}`, ruleId, mode, at: 1_000 - n, sessionId: "s" });

/** Activity's list as a poll leaves it: `rows` loaded, `total` in range, the cursor `next` past the last row. */
const loaded = (rows: Violation[], total: number, next: string | null) =>
  ({ ...pagedList<Violation>("/api/violations?since=0&limit=200"), rows, total, next, pages: 1 });

afterEach(() => {
  data.rules = new Map();
  data.activity = {};
  data.activityAll = {};
  data.activitySessions = null;
  data.findings = pagedList();
  ui.actPack = "";
  ui.actRule = null;
  ui.recentShown = 50;
  vi.unstubAllGlobals();
});

describe("past violations keep the mode they were recorded with", () => {
  it("leaves a rule's steered violations out of logged-only after it is switched to Log", () => {
    // Two steered violations recorded while in Steer, one logged after the switch to Log.
    data.rules = new Map([["r", storedRule("r", "log", true)]]);
    data.activity = { r: { ...NONE, distinctViolations: 3, loggedViolations: 1, rerails: 2, audits: 3 } };

    expect(summary()).toMatchObject({ triggers: 3, logged: 1 });
  });

  it("labels a past violation by its recorded mode in the timeline and Activity", () => {
    expect(violationOutcome({ mode: "steer" })).toBe("steered");
    expect(violationOutcome({ mode: "log" })).toBe("logged");
  });
});

describe("overview session tiles", () => {
  it("read the activity answer's session counts, not the loaded lists", () => {
    data.activitySessions = { started: 12, withViolations: 5 };
    data.findings = loaded([finding(1, "a")], 1, null);

    expect(summary()).toMatchObject({ sessionsInRange: 12, sessionsWithTriggers: 5 });
  });
});

describe("whether a rule has been audited", () => {
  it("is yes for a rule that ran clean and was then switched off", () => {
    data.rules = new Map([["r", storedRule("r", "steer", false)]]);
    data.activityAll = { r: { ...NONE, audits: 4 } };

    expect(isAudited("r")).toBe(true);
  });

  it("is no for a rule switched on that no audit has run yet", () => {
    data.rules = new Map([["r", storedRule("r", "steer", true)]]);
    data.activityAll = { r: NONE };

    expect(isAudited("r")).toBe(false);
  });
});

describe("Activity filtered to one rule", () => {
  beforeEach(() => {
    data.rules = new Map([["a", storedRule("a", "steer", true)], ["b", storedRule("b", "steer", true)]]);
    ui.actRule = "b";
  });

  it("keeps Show more while the server has pages the filter has not seen, and is not empty yet", () => {
    data.findings = loaded(Array.from({ length: 50 }, (_, n) => finding(n, "a")), 120, "c50");

    const view = activityView();

    expect(view.none).toBe(false);
    expect(view.more).toBeTruthy();
  });

  it("fetches further pages by cursor on Show more until the filter has enough matches or the range runs out", async () => {
    data.findings = loaded(Array.from({ length: 50 }, (_, n) => finding(n, "a")), 150, "c50");
    const pages: Record<string, { violations: Violation[]; nextCursor: string | null }> = {
      c50: { violations: Array.from({ length: 50 }, (_, n) => finding(50 + n, "a")), nextCursor: "c100" },
      c100: { violations: Array.from({ length: 50 }, (_, n) => finding(100 + n, n < 3 ? "b" : "a")), nextCursor: null },
    };
    const requested: URL[] = [];
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("fetch", async (input: URL) => {
      const url = new URL(input);
      requested.push(url);
      return new Response(JSON.stringify({ ...pages[url.searchParams.get("cursor") ?? ""], total: 150 }), { status: 200 });
    });

    await showMoreActivity();

    expect(requested.map(url => url.searchParams.get("cursor"))).toEqual(["c50", "c100"]);
    expect(requested.some(url => url.searchParams.has("offset"))).toBe(false);
    const view = activityView();
    expect(view.shown).toHaveLength(3);
    expect(view.none).toBe(false);
    expect(view.more).toBe(false);
  });
});
