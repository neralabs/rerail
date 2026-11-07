import { afterEach, describe, expect, it } from "vitest";
import { lastEventAt, openState, STALE_MIN, violationCounts, type SessionRow } from "../../ui/sessions-model.js";
import { data } from "../../ui/state.js";

const MIN = 60_000;
const NOW = 1_800_000_000_000;

const row = (fields: Partial<SessionRow> = {}): SessionRow => ({ sessionId: "s1", startedAt: NOW - 120 * MIN, endedAt: null, lastEventAt: null, ...fields });

function loadTranscript(sessionId: string, observedAt: number) {
  data.sessionDetail.set(sessionId, {
    head: null, gaps: null, events: [{ sequence: 1, observedAt }], nextAfter: 1, loaded: true, earlier: false, loading: null,
  });
}

afterEach(() => data.sessionDetail.clear());

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
  it("reports logged and steered from the row", () => {
    expect(violationCounts(row({ violationCount: 7, steeredViolations: 3 }))).toEqual({ logged: 7, steered: 3 });
  });

  it("reads zero for a row without counts", () => {
    expect(violationCounts(row())).toEqual({ logged: 0, steered: 0 });
  });

  it("never reports more steered than logged", () => {
    expect(violationCounts(row({ violationCount: 2, steeredViolations: 5 }))).toEqual({ logged: 2, steered: 2 });
  });
});
