import { afterEach, describe, expect, it } from "vitest";
import { data, isAudited } from "../../ui/state.js";
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

afterEach(() => {
  data.rules = new Map();
  data.activityAll = {};
});

describe("past violations keep the outcome they were recorded with", () => {
  it("labels a past violation by whether the agent was told, not by its rule's mode", () => {
    expect(violationOutcome({ mode: "steer", logged: false })).toBe("steered");
    expect(violationOutcome({ mode: "log", logged: true })).toBe("logged");
    // An audit in Log mode told the agent nothing, whatever the rule's mode.
    expect(violationOutcome({ mode: "steer", logged: true })).toBe("logged");
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

