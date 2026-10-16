import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { loadEarlier, loadMoreFindings, loadSession, loadSessionFindings, refresh, refreshNow } from "../../ui/poll.js";
import { placeCorrections, sessionCorrections, sessionTurns } from "../../ui/sessions-model.js";
import { data, pagedList, setScope, ui, type Violation } from "../../ui/state.js";

/** How the test daemon answers one GET: the body, a `Response` (another status), or a promise of either. */
type Answer = (url: URL) => unknown;

let requested: URL[] = [];
let answer: Answer = () => ({});

const violation = (n: number, sessionId = "s1", at = 1_000 - n): Violation => ({ id: `v${n}`, auditId: `a${n}`, ruleId: "r", sessionId, at, mode: "steer" });
const ids = (rows: readonly Violation[]) => rows.map(row => row.id);
const event = (sequence: number) => ({ sequence, sessionId: "s1", eventId: `e${sequence}`, kind: "assistant-message", observedAt: 10_000 + sequence, payload: { text: "…" } });
const session = { sessionId: "s1", host: "claude-code", cwd: "/w", repositoryId: "A", startedAt: 1, endedAt: null, eventCount: 510, violationCount: 0 };

beforeEach(() => {
  requested = [];
  answer = () => ({});
  vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
  vi.stubGlobal("fetch", async (input: URL) => {
    const url = new URL(input);
    requested.push(url);
    const answered = await answer(url);
    return answered instanceof Response ? answered : new Response(JSON.stringify(answered), { status: 200 });
  });
  setScope({ repository: "A" });
  data.repository = { setUp: true, repositoryKey: "A" };
  data.rules = new Map([["r", { rule: { id: "r" } }]]);
  Object.assign(data, { findings: pagedList(), ruleFindings: pagedList(), sessions: pagedList(), sessionFindings: pagedList() });
  data.sessionDetail.clear();
  ui.panel = null;
  ui.selectedSession = null;
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

/** A daemon with every resource a poll may read, each empty. */
function emptyDaemon(url: URL): unknown {
  if (url.pathname === "/healthz") return { version: "1", pid: 1 };
  if (url.pathname === "/api/repositories") return { repositories: [] };
  if (url.pathname === "/api/repository") return { repository: data.repository, checkout: null };
  if (url.pathname === "/api/repository/rules") return { rules: [], unavailable: [] };
  if (url.pathname === "/api/rule-packs") return { packs: [] };
  if (url.pathname === "/api/rules/activity") return { since: 0, rules: {}, sessions: { started: 0, withViolations: 0 } };
  if (url.pathname === "/api/violations") return { violations: [], total: 0, nextCursor: null };
  if (url.pathname === "/api/sessions") return { sessions: [], total: 0, nextCursor: null };
  if (url.pathname === "/api/sessions/s1") return { session };
  if (url.pathname === "/api/sessions/s1/events") return { events: [], nextAfter: 0, pageLimit: 500 };
  throw new Error(`unexpected ${url.pathname}`);
}

const paths = () => [...new Set(requested.map(url => url.pathname))].sort();

describe("a poll", () => {
  beforeEach(() => { answer = emptyDaemon; });

  it("reads only what Activity shows, besides the shared reads", async () => {
    ui.view = "activity";

    await refresh();

    expect(paths()).toEqual(["/api/repositories", "/api/repository", "/api/repository/rules", "/api/rule-packs", "/api/violations", "/healthz"]);
  });

  it("reads only the sessions list and the selected session on Sessions", async () => {
    ui.view = "sessions";
    ui.selectedSession = "s1";

    await refresh();

    expect(paths()).toEqual(["/api/repositories", "/api/repository", "/api/repository/rules", "/api/sessions", "/api/sessions/s1", "/api/sessions/s1/events", "/api/violations", "/healthz"]);
    const violations = requested.find(url => url.pathname === "/api/violations")!;
    expect(violations.searchParams.get("sessionId")).toBe("s1");
    expect(violations.searchParams.get("since")).toBe("0");
  });

  it("reads nothing beyond the shared reads on Settings", async () => {
    ui.view = "settings";

    await refresh();

    expect(paths()).toEqual(["/api/repositories", "/api/repository", "/healthz"]);
  });

  it("is not scheduled while the tab is hidden, and is while it shows", async () => {
    ui.view = "settings";
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    vi.stubGlobal("document", { hidden: true });

    await refreshNow();
    const afterFirst = requested.length;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(requested.length).toBe(afterFirst);

    vi.stubGlobal("document", { hidden: false });
    await refreshNow();
    const visible = requested.length;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(requested.length).toBeGreaterThan(visible);
  });
});

describe("Load more on a paged list", () => {
  it("appends the next cursor page without duplicates, and a first-page refresh keeps the loaded tail and its cursor", async () => {
    ui.view = "activity";
    let first = { violations: [violation(1), violation(2)], total: 3, nextCursor: "c2" };
    answer = url => {
      if (url.pathname !== "/api/violations") return emptyDaemon(url);
      const cursor = url.searchParams.get("cursor");
      if (cursor === "c2") return { violations: [violation(2), violation(3)], total: 3, nextCursor: "c3" };
      return first;
    };
    await refresh();

    await loadMoreFindings();

    expect(ids(data.findings.rows)).toEqual(["v1", "v2", "v3"]);
    expect(data.findings.next).toBe("c3");

    // A new violation arrives: the first page shifts, and its own cursor points into what is already loaded.
    first = { violations: [violation(0), violation(1)], total: 4, nextCursor: "c1" };
    await refresh();

    expect(ids(data.findings.rows)).toEqual(["v0", "v1", "v2", "v3"]);
    expect(data.findings.next).toBe("c3");
    expect(data.findings.total).toBe(4);
  });

  it("starts over from the fresh first page when more rows arrived than it reaches, and takes a complete page whole", async () => {
    ui.view = "activity";
    let first: { violations: Violation[]; total: number; nextCursor: string | null } = { violations: [violation(5), violation(6)], total: 4, nextCursor: "c6" };
    answer = url => {
      if (url.pathname !== "/api/violations") return emptyDaemon(url);
      return url.searchParams.get("cursor") === "c6" ? { violations: [violation(7), violation(8)], total: 4, nextCursor: null } : first;
    };
    await refresh();
    await loadMoreFindings();
    expect([ids(data.findings.rows), data.findings.next]).toEqual([["v5", "v6", "v7", "v8"], null]);

    // Two more violations than the page holds: v3 and v4 lie between the fresh page and the kept rows. Keeping the
    // old (null) cursor would leave them unreachable; the list starts over with the fresh page's own cursor.
    first = { violations: [violation(1), violation(2)], total: 8, nextCursor: "c2" };
    await refresh();
    expect([ids(data.findings.rows), data.findings.next, data.findings.pages]).toEqual([["v1", "v2"], "c2", 1]);

    // A first page that is the whole list replaces what was loaded, rows deleted since included.
    await loadMoreFindings();
    first = { violations: [violation(1)], total: 1, nextCursor: null };
    await refresh();
    expect([ids(data.findings.rows), data.findings.next]).toEqual([["v1"], null]);
  });
});

describe("the selected session's violations", () => {
  it("are dropped when another session is selected, and a late answer for the previous one lands nowhere", async () => {
    let held: Promise<unknown> | null = null;
    answer = url => {
      const sessionId = url.searchParams.get("sessionId")!;
      return sessionId === "s1" && held !== null ? held : { violations: [violation(1, sessionId)], total: 1, nextCursor: null };
    };
    data.sessionDetail.set("s1", { head: null, gaps: null, events: [], nextAfter: 0, loaded: true, earlier: false, loading: null });
    await loadSessionFindings("s1");
    expect(sessionCorrections("s1")).toHaveLength(1);

    // s1's next refresh is still out when s2 is selected.
    const late = Promise.withResolvers<unknown>();
    held = late.promise;
    const refreshOfS1 = loadSessionFindings("s1");
    const s2 = loadSessionFindings("s2");

    expect(data.sessionFindings.owner).toBe("s2");
    expect(data.sessionFindings.rows).toEqual([]);
    expect(sessionCorrections("s1")).toEqual([]);
    await s2;
    late.resolve({ violations: [violation(9)], total: 1, nextCursor: null });
    await refreshOfS1;
    expect(data.sessionFindings.rows.map(row => row.sessionId)).toEqual(["s2"]);
  });
});

describe("a session's transcript", () => {
  /** A session of 1,010 events and 300 violations, the newest violation at the last event. */
  function sessionDaemon(url: URL): unknown {
    if (url.pathname === "/api/sessions/s1") return { session: { ...session, eventCount: 1_010 } };
    if (url.pathname === "/api/sessions/s1/events") {
      const before = url.searchParams.get("before"), after = Number(url.searchParams.get("after") ?? 0);
      const all = Array.from({ length: 1_010 }, (_, i) => event(i + 1));
      const page = before === null ? all.filter(e => e.sequence > after).slice(0, 500) : all.filter(e => e.sequence < Number(before)).slice(-500);
      return { events: page, nextAfter: page.at(-1)?.sequence ?? after, pageLimit: 500 };
    }
    if (url.pathname === "/api/violations") {
      const offset = Number(url.searchParams.get("cursor") ?? 0), limit = Number(url.searchParams.get("limit"));
      const rows = Array.from({ length: 300 }, (_, n) => violation(n, "s1", 11_010 - n * 3)).slice(offset, offset + limit);
      return { violations: rows, total: 300, nextCursor: offset + limit < 300 ? String(offset + limit) : null };
    }
    throw new Error(`unexpected ${url.pathname}`);
  }
  const eventQueries = () => requested.filter(url => url.pathname.endsWith("/events")).map(url => url.search.replace(/&limit=500$/u, ""));
  const sequences = () => data.sessionDetail.get("s1")!.events.map(e => e.sequence);

  it("opens on the newest events, and Load earlier reaches back through events and violations together", async () => {
    answer = sessionDaemon;
    await loadSession("s1");
    await loadSessionFindings("s1");

    expect(eventQueries()).toEqual([`?before=${Number.MAX_SAFE_INTEGER}`]);
    expect([sequences()[0], sequences().at(-1)]).toEqual([511, 1_010]);
    // Only violations at or after the oldest loaded event attach: older ones belong to events not loaded yet.
    expect(sessionCorrections("s1")).toHaveLength(167);

    // The events reach back less far than the violations: an earlier events page.
    await loadEarlier("s1");
    expect(eventQueries().at(-1)).toBe("?before=511");
    expect(sequences()[0]).toBe(11);
    // Now the violations limit the window, up to (not including) their oldest loaded audit start.
    expect(sessionCorrections("s1")).toHaveLength(199);

    // The violations reach back less far: their next page, and no events request.
    await loadEarlier("s1");
    expect(eventQueries()).toHaveLength(2);
    expect(data.sessionFindings.rows).toHaveLength(300);
    expect(sessionCorrections("s1")).toHaveLength(300);

    await loadEarlier("s1");
    expect(sequences()).toHaveLength(1_010);
    expect(data.sessionDetail.get("s1")!.earlier).toBe(false);
    expect(await loadEarlier("s1")).toBe(false);

    // A poll appends what the session records next.
    await loadSession("s1");
    expect(eventQueries().at(-1)).toBe("?after=1010");
  });
});

describe("the selected session", () => {
  beforeEach(() => { ui.view = "sessions"; });

  it("refreshes its row in the list every poll, even on a page past the first", async () => {
    // s1 came in on a later page: the first-page refresh (newest session s0, its cursor reaching the kept rows) never
    // rewrites it, its head does.
    const s0 = { ...session, sessionId: "s0", startedAt: 2 };
    data.sessions = { ...pagedList("/api/sessions?limit=50&repository=A"), rows: [s0, { ...session, eventCount: 5 }], total: 60, next: "c1", pages: 2 };
    let eventCount = 5;
    answer = url => {
      if (url.pathname === "/api/sessions") return { sessions: [s0], total: 60, nextCursor: "c0" };
      return url.pathname === "/api/sessions/s1" ? { session: { ...session, eventCount } } : emptyDaemon(url);
    };
    ui.selectedSession = "s1";
    await refresh();
    eventCount = 9;
    await refresh();

    expect(data.sessions.rows).toEqual([expect.objectContaining({ sessionId: "s0" }), expect.objectContaining({ sessionId: "s1", eventCount: 9 })]);
  });

  it("drops the selection when the session goes away, instead of reading as an unknown repository", async () => {
    let gone = false;
    answer = url => (url.pathname === "/api/sessions/s1" && gone
      ? new Response(JSON.stringify({ error: "session-not-found" }), { status: 404 })
      : emptyDaemon(url));
    ui.selectedSession = "s1";
    await refresh();
    expect(data.sessionDetail.get("s1")?.loaded).toBe(true);

    gone = true;
    await refresh();

    expect(data.missing).toBe(false);
    expect(data.disconnected).toBe(false);
    expect(ui.selectedSession).toBeNull();
    expect(data.sessionDetail.has("s1")).toBe(false);
  });

  it("keeps a violation whose rule is no longer in the repository", async () => {
    data.sessionDetail.set("s1", { head: null, gaps: null, events: [], nextAfter: 0, loaded: true, earlier: false, loading: null });
    data.sessionFindings = { ...pagedList<Violation>("k", "s1"), rows: [violation(1), { ...violation(2), ruleId: "removed" }], total: 2, pages: 1 };

    expect(sessionCorrections("s1").map(c => [c.finding.ruleId, c.rule === null])).toEqual([["r", false], ["removed", true]]);
  });
});

describe("a selected session's counts and timeline", () => {
  it("writes its violation total into its list row, past the first page too, so every count of it agrees", async () => {
    data.sessions = { ...pagedList("/api/sessions?limit=50&repository=A"), rows: [{ ...session, violationCount: 0 }], total: 60, next: "c1", pages: 2 };
    answer = url => (url.pathname === "/api/violations" ? { violations: [violation(1)], total: 1, nextCursor: null } : emptyDaemon(url));

    await loadSessionFindings("s1");

    expect(data.sessions.rows).toEqual([expect.objectContaining({ sessionId: "s1", violationCount: 1 })]);
  });

  it("keeps its violations when the loaded window starts between a tool call and its result", () => {
    // Only the result and the turn end of the call that opened the turn are loaded: they make no turn of their own.
    const events = [
      { sequence: 11, sessionId: "s1", eventId: "r", kind: "result", observedAt: 110, payload: { toolCallId: "t1", failed: false } },
      { sequence: 12, sessionId: "s1", eventId: "e", kind: "turn-end", observedAt: 120, payload: {} },
    ];
    const corrections = [{ key: "a1:v1", finding: { ...violation(1), path: "src/a.ts" }, rule: null }];

    const turns = placeCorrections(sessionTurns(events), corrections as never);

    expect(turns.flatMap(turn => turn.orphans.map(c => c.key))).toEqual(["a1:v1"]);
  });
});
