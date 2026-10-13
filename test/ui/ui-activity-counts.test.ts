import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { activityView, data, isAudited, sinceFor, summary, ui } from "../../ui/state.js";
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

const finding = (n: number, ruleId: string, mode = "steer") => ({ id: `v${n}`, auditId: `a${n}`, ruleId, mode, at: 1_000 - n, sessionId: "s" });

afterEach(() => {
  data.rules = new Map();
  data.activity = {};
  data.activityAll = {};
  data.findings = [];
  data.findingsTotal = 0;
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
    data.findings = Array.from({ length: 50 }, (_, n) => finding(n, "a"));
    data.findingsTotal = 120;

    const view = activityView();

    expect(view.none).toBe(false);
    expect(view.more).toBeTruthy();
  });

  it("fetches further pages on Show more until the filter has enough matches or the range runs out", async () => {
    data.findings = Array.from({ length: 50 }, (_, n) => finding(n, "a"));
    data.findingsTotal = 150;
    data.findingsSince = sinceFor();
    const pages: Record<number, ReturnType<typeof finding>[]> = {
      50: Array.from({ length: 50 }, (_, n) => finding(50 + n, "a")),
      100: Array.from({ length: 50 }, (_, n) => finding(100 + n, n < 3 ? "b" : "a")),
    };
    const offsets: number[] = [];
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("fetch", async (input: URL) => {
      const offset = Number(new URL(input).searchParams.get("offset"));
      offsets.push(offset);
      return new Response(JSON.stringify({ violations: pages[offset] ?? [], total: 150, offset, limit: 50 }), { status: 200 });
    });

    await showMoreActivity();

    expect(offsets).toEqual([50, 100]);
    const view = activityView();
    expect(view.shown).toHaveLength(3);
    expect(view.none).toBe(false);
    expect(view.more).toBe(false);
  });
});
