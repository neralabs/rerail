import { afterEach, describe, expect, it } from "vitest";
import { coverageGaps, placesHeading } from "../../ui/render/timeline.js";
import { discoveryGapPlaces, lastEventAt, openState, sessionCorrections, sessionDeclinedTotal, STALE_MIN, violationCounts, type SessionRow } from "../../ui/sessions-model.js";
import { data, pagedList } from "../../ui/state.js";

const MIN = 60_000;
const NOW = 1_800_000_000_000;

const row = (fields: Partial<SessionRow> = {}): SessionRow => ({ sessionId: "s1", startedAt: NOW - 120 * MIN, endedAt: null, lastEventAt: null, ...fields });

function loadTranscript(sessionId: string, observedAt: number) {
  data.sessionDetail.set(sessionId, {
    head: null, gaps: null, events: [{ sequence: 1, observedAt }], nextAfter: 1, loaded: true, earlier: false, loading: null,
  });
}

afterEach(() => {
  data.sessionDetail.clear();
  data.sessionFindings = pagedList();
});

describe("live or idle, from the list row's lastEventAt", () => {
  it("is live up to STALE_MIN after the last event and idle (no end recorded) one minute past it", () => {
    expect(openState(row({ lastEventAt: NOW - STALE_MIN * MIN }), NOW)).toBe("live");
    expect(openState(row({ lastEventAt: NOW - (STALE_MIN + 1) * MIN }), NOW)).toBe("stale");
  });

  it("reads the row's last event without the transcript loaded, not the session start", () => {
    const s = row({ startedAt: NOW - 600 * MIN, lastEventAt: NOW - 2 * MIN });
    expect(lastEventAt(s)).toBe(NOW - 2 * MIN);
    expect(openState(s, NOW)).toBe("live");
  });

  it("takes the loaded transcript's newest event when the row lags a poll behind it", () => {
    const s = row({ lastEventAt: NOW - 90 * MIN });
    loadTranscript("s1", NOW - MIN);
    expect(openState(s, NOW)).toBe("live");
  });

  it("keeps the row's time when the loaded transcript page is older than it", () => {
    loadTranscript("s1", NOW - 90 * MIN);
    expect(lastEventAt(row({ lastEventAt: NOW - 5 * MIN }))).toBe(NOW - 5 * MIN);
  });

  it("falls back to the start for a session with no events yet", () => {
    expect(lastEventAt(row({ startedAt: NOW - 3 * MIN }))).toBe(NOW - 3 * MIN);
    expect(openState(row({ startedAt: NOW - 3 * MIN }), NOW)).toBe("live");
  });

  it("has no live state once the session ended, however recent its last event", () => {
    expect(openState(row({ endedAt: NOW - MIN, lastEventAt: NOW - MIN }), NOW)).toBeNull();
  });
});

describe("violationCounts", () => {
  it("reports logged, steered and declined from the row", () => {
    expect(violationCounts(row({ violationCount: 7, steeredViolations: 3, declinedViolations: 1 }))).toEqual({ logged: 7, steered: 3, declined: 1 });
  });

  it("reads zero for a row without counts", () => {
    expect(violationCounts(row())).toEqual({ logged: 0, steered: 0, declined: 0 });
  });

  it("never reports more steered than logged, or more declined than steered", () => {
    expect(violationCounts(row({ violationCount: 2, steeredViolations: 5, declinedViolations: 9 }))).toEqual({ logged: 2, steered: 2, declined: 2 });
  });
});

describe("declined steers", () => {
  const finding = (id: string, auditId: string, ruleId: string, logged = false) => ({ id, auditId, ruleId, logged, at: 100, sessionId: "s1" });
  const note = (ruleId: string, auditId: string | null, reason: string) => ({ ruleId, auditId, reason, recordedAt: 200 });

  function loadSession(notes: ReturnType<typeof note>[], violations = notes.length) {
    data.sessionDetail.set("s1", {
      head: null, gaps: null, declined: { violations, notes }, events: [], nextAfter: 0, loaded: true, earlier: false, loading: null,
    });
    data.sessionFindings = { ...pagedList(null, "s1"), pages: 1, rows: [
      finding("v1", "a1", "complexity"),
      finding("v2", "a1", "complexity"),
      finding("v3", "a2", "complexity"),
      finding("v4", "a1", "no-any"),
      finding("v5", "a1", "logged-rule", true),
    ] };
  }

  it("gives every steered card of the declined audit and rule the agent's reasons, oldest first, and no other card any", () => {
    loadSession([note("complexity", "a1", "already at 14 on main"), note("complexity", "a1", "only adds a parameter"), note("no-any", null, "never placed")]);
    expect(Object.fromEntries(sessionCorrections("s1").map(c => [c.finding.id, c.declined]))).toEqual({
      v1: ["already at 14 on main", "only adds a parameter"],
      v2: ["already at 14 on main", "only adds a parameter"],
      v3: null,
      v4: null,
      v5: null,
    });
  });

  it("never marks a logged card declined, whatever note names its audit and rule", () => {
    loadSession([note("logged-rule", "a1", "not told, so nothing to decline")]);
    expect(sessionCorrections("s1").find(c => c.finding.id === "v5")?.declined).toBeNull();
  });

  it("counts the header's declined from the loaded session, else from the list row", () => {
    expect(sessionDeclinedTotal(row({ violationCount: 4, steeredViolations: 3, declinedViolations: 1 }))).toBe(1);
    loadSession([note("complexity", "a1", "reason")], 2);
    expect(sessionDeclinedTotal(row({ violationCount: 4, steeredViolations: 3, declinedViolations: 1 }))).toBe(2);
  });
});

describe("what the timeline says Rerail did not see of the session", () => {
  const NO_GAPS = { droppedEvents: 0, uncheckedBoundaries: 0, codeRulesPendingBoundaries: 0, ignoredEdits: 0, ignoreRuleChanges: 0 };
  function withGaps(gaps: object) {
    data.sessionDetail.set("s1", { head: null, gaps, events: [], nextAfter: 0, loaded: true, earlier: false, loading: null });
  }

  it("says each count in its words and its order, and nothing for a session it saw whole", () => {
    withGaps(NO_GAPS);
    expect(coverageGaps(row())).toBe("");
    withGaps({ droppedEvents: 1, uncheckedBoundaries: 1, codeRulesPendingBoundaries: 1, ignoredEdits: 1, ignoreRuleChanges: 1 });
    expect(coverageGaps(row())).toBe([
      "1 event never reached Rerail", "rules had not yet seen shell edits at 1 boundary", "code rules were not yet running at 1 boundary",
      "1 edit went unchecked because Git ignores the file", "Git's ignore rules changed once",
    ].join("; "));
    withGaps({ droppedEvents: 1200, uncheckedBoundaries: 2, codeRulesPendingBoundaries: 3, ignoredEdits: 4, ignoreRuleChanges: 5 });
    expect(coverageGaps(row())).toBe([
      "1,200 events never reached Rerail", "rules had not yet seen shell edits at 2 boundaries",
      "code rules were not yet running at 3 boundaries", "4 edits went unchecked because Git ignores the file", "Git's ignore rules changed 5 times",
    ].join("; "));
  });

  it("says when rules could not fully check part of the repository, beside the boundaries that answered before their walk ended", () => {
    withGaps({ ...NO_GAPS, discoveryGaps: 1 });
    expect(coverageGaps(row())).toBe("rules could not fully check part of the repository (1 place)");
    withGaps({ ...NO_GAPS, uncheckedBoundaries: 2, discoveryGaps: 3 });
    expect(coverageGaps(row())).toBe("rules had not yet seen shell edits at 2 boundaries; rules could not fully check part of the repository (3 places)");
  });

  it("lists the places the daemon names with why, and says how many of the session's places they are", () => {
    const place = { boundary: "turn", path: "src/locked", reason: "EACCES: permission denied, opendir '/repo/src/locked'", observedAt: 1 };
    withGaps({ ...NO_GAPS, discoveryGaps: 1, discoveryGapPlaces: [place] });
    expect(discoveryGapPlaces(row())).toEqual([place]);
    withGaps({ ...NO_GAPS, discoveryGaps: 1 });
    expect(discoveryGapPlaces(row())).toEqual([]);
    // Words true of every reason a place has: one Rerail could not read went unchecked, one checked in part was not.
    expect(placesHeading(row(), 1)).toBe("Rules could not fully check this place:");
    withGaps({ ...NO_GAPS, discoveryGaps: 3 });
    expect(placesHeading(row(), 3)).toBe("Rules could not fully check these places:");
    withGaps({ ...NO_GAPS, discoveryGaps: 1000 });
    expect(placesHeading(row(), 20)).toBe("Rules could not fully check the first 20 of these 1,000 places:");
  });

  it("says how many places the record holds and that the check met more, once it met more than the record takes", () => {
    // 1,001, 1,500 or 10,000 places met: the record holds the first 1,000, and one more record, no place, says it met more.
    withGaps({ ...NO_GAPS, discoveryGaps: 1000, discoveryGapsMore: true });
    expect(coverageGaps(row())).toBe("rules could not fully check part of the repository (more than 1,000 places; 1,000 recorded)");
    expect(placesHeading(row(), 20)).toBe("Rules could not fully check more than 1,000 places; the first 20 of the 1,000 recorded:");
    withGaps({ ...NO_GAPS, discoveryGaps: 1000, discoveryGapsMore: false });
    expect(coverageGaps(row())).toBe("rules could not fully check part of the repository (1,000 places)");
    expect(placesHeading(row(), 20)).toBe("Rules could not fully check the first 20 of these 1,000 places:");
  });

  it("says nothing of a walk's gaps when the session has none, or the answer counts none", () => {
    withGaps({ ...NO_GAPS, discoveryGaps: 0 });
    expect(coverageGaps(row())).toBe("");
    withGaps(NO_GAPS);
    expect(coverageGaps(row())).toBe("");
  });
});
