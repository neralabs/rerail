import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SqliteSessionReader, SqliteSessionStore } from "../../../src/runtime/session-store.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
const readers: SqliteSessionReader[] = [];

afterEach(async () => {
  for (const reader of readers.splice(0)) reader.close();
  vi.restoreAllMocks();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

/** Spies on this thread's stderr, where the reader writes the line that names a damaged event: what it holds so far. */
function spyOnStderr(): () => string {
  const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
  return () => stderr.mock.calls.map(([chunk]) => String(chunk)).join("");
}

/**
 * Two sessions, a and b, each tracked by the HUD from its start, with rule coverage, events its host dropped (2, then
 * 3) and steers: a's `receipt-a1` of rule-a and `receipt-a2` of rule-a and rule-c, b's `receipt-b1` of rule-b.
 */
async function twoSessions(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-damaged-event-"));
  temporaryDirectories.push(root);
  const path = join(root, "rerail.sqlite");
  const store = new SqliteSessionStore(path);
  try {
    for (const [id, steers] of [["a", [["rule-a"], ["rule-a", "rule-c"]]], ["b", [["rule-b"]]]] as const) {
      const session = { host: "claude-code" as const, sessionId: `session-${id}`, cwd: root };
      store.openSession(session, Date.now(), `repo-${id}`);
      store.startHudTracking(session, true);
      store.recordRuleCoverage(session, { repositoryId: `repo-${id}`, ownerCheckoutPaths: [join(root, id)], activeRuleCount: 1, ruleSignature: "s" });
      steers.forEach((ruleIds, index) => store.recordSteer(session, `receipt-${id}${index + 1}`, ruleIds));
      for (const [n, count] of [[1, 2], [2, 3]] as const) {
        store.recordDroppedEvents(session, {
          id: `drop-${n}`, count, kinds: { call: count }, firstEventId: null, lastEventId: null, firstDroppedAt: n, lastDroppedAt: n,
        });
      }
    }
  } finally {
    store.close();
  }
  return path;
}

/** Writes `edit` of the stored text of session `sessionId`'s event `eventId` over it, as a damaged file or a hand edit leaves it. */
function damage(path: string, sessionId: string, eventId: string, edit: (text: string) => string): void {
  const client = new Database(path);
  try {
    const row = client.prepare("SELECT payload_json AS text FROM events WHERE session_id = ? AND event_id = ?").get(sessionId, eventId) as { text: string };
    client.prepare("UPDATE events SET payload_json = ? WHERE session_id = ? AND event_id = ?").run(edit(row.text), sessionId, eventId);
  } finally {
    client.close();
  }
}

/** The event id of session `sessionId`'s newest `kind` event. */
function eventIdOf(path: string, sessionId: string, kind: string): string {
  const client = new Database(path, { readonly: true });
  try {
    return (client.prepare("SELECT event_id AS id FROM events WHERE session_id = ? AND kind = ? ORDER BY sequence DESC LIMIT 1").get(sessionId, kind) as { id: string }).id;
  } finally {
    client.close();
  }
}

/** One byte changed: the first `[` after `key` made `{`, as the issue's damage.mjs changes it in the file. */
const byteAfter = (key: string) => (text: string): string => text.replace(`"${key}":[`, `"${key}":{`);

function reader(path: string): SqliteSessionReader {
  const opened = new SqliteSessionReader(path);
  readers.push(opened);
  return opened;
}

/** The lines of `logged` that name session `sessionId`'s event `eventId`. */
function namingLines(logged: string, sessionId: string, eventId: string): string[] {
  return logged.split("\n").filter(line => line.includes(sessionId) && line.includes(eventId));
}

describe("a session event whose stored text does not read", () => {
  it("costs the session page its coverage alone, and the log names the event once", async () => {
    const path = await twoSessions();
    const coverage = eventIdOf(path, "session-a", "rule-coverage");
    damage(path, "session-a", coverage, byteAfter("ownerCheckoutPaths"));
    const logged = spyOnStderr();
    const sessions = reader(path);

    expect(sessions.hud("session-a")).toEqual({ activeRuleCount: null, steeringCount: 3, overlays: [], awaitingApproval: null });
    expect(sessions.hud("session-a")).toEqual({ activeRuleCount: null, steeringCount: 3, overlays: [], awaitingApproval: null });
    expect(sessions.hud("session-b").steeringCount).toBe(1);
    expect(namingLines(logged(), "session-a", coverage)).toHaveLength(1);
  });

  it("takes coverage whose owner paths are JSON but no list of strings as none", async () => {
    const path = await twoSessions();
    const coverage = eventIdOf(path, "session-a", "rule-coverage");
    damage(path, "session-a", coverage, text => JSON.stringify({ ...JSON.parse(text) as object, ownerCheckoutPaths: "not a list" }));
    const logged = spyOnStderr();

    expect(reader(path).hud("session-a")).toEqual({ activeRuleCount: null, steeringCount: 3, overlays: [], awaitingApproval: null });
    expect(namingLines(logged(), "session-a", coverage)).toHaveLength(1);
  });

  it("leaves a damaged steer receipt out of the session's steered count and the rules activity, named once", async () => {
    const path = await twoSessions();
    damage(path, "session-a", "steering-delivered:receipt-a2", byteAfter("ruleIds"));
    const logged = spyOnStderr();
    const sessions = reader(path);

    expect(sessions.hud("session-a").steeringCount).toBe(1);
    expect(Object.fromEntries(sessions.steersByRule(0))).toEqual({ "rule-a": 1, "rule-b": 1 });
    expect(Object.fromEntries(sessions.steersByRule(0, ["repo-b"]))).toEqual({ "rule-b": 1 });
    expect(namingLines(logged(), "session-a", "steering-delivered:receipt-a2")).toHaveLength(1);
  });

  it("reads a receipt whose rule ids are JSON but no list as a steer that names no rule", async () => {
    const path = await twoSessions();
    damage(path, "session-a", "steering-delivered:receipt-a2", text => text.replace(/"ruleIds":\[[^\]]*\]/u, `"ruleIds":"rule-c"`));
    const logged = spyOnStderr();
    const sessions = reader(path);

    expect(Object.fromEntries(sessions.steersByRule(0))).toEqual({ "rule-a": 1, "rule-b": 1 });
    expect(sessions.hud("session-a").steeringCount).toBe(2);
    expect(logged()).toBe("");
  });

  it("gives no steered count for a session whose HUD tracking start does not read", async () => {
    const path = await twoSessions();
    damage(path, "session-a", "hud-tracking-start", text => text.slice(0, -1));
    const logged = spyOnStderr();

    expect(reader(path).hud("session-a").steeringCount).toBeNull();
    expect(namingLines(logged(), "session-a", "hud-tracking-start")).toHaveLength(1);
  });

  it("counts the dropped events of the records that read", async () => {
    const path = await twoSessions();
    damage(path, "session-a", "events-dropped:drop-2", text => text.replace(`"count":3`, `"count":3x`));
    damage(path, "session-b", "events-dropped:drop-1", text => text.replace(`"count":2`, `"count":"2"`));
    const logged = spyOnStderr();
    const sessions = reader(path);

    expect(sessions.gaps("session-a").droppedEvents).toBe(2);
    expect(sessions.gaps("session-a").droppedEvents).toBe(2);
    expect(sessions.gaps("session-b").droppedEvents).toBe(3);
    expect(namingLines(logged(), "session-a", "events-dropped:drop-2")).toHaveLength(1);
    expect(namingLines(logged(), "session-b", "events-dropped:drop-1")).toHaveLength(1);
  });

  it("reads every figure as before when every event reads", async () => {
    const path = await twoSessions();
    const logged = spyOnStderr();
    const sessions = reader(path);

    expect(sessions.hud("session-a").steeringCount).toBe(3);
    expect(Object.fromEntries(sessions.steersByRule(0))).toEqual({ "rule-a": 2, "rule-b": 1, "rule-c": 1 });
    expect(sessions.gaps("session-a").droppedEvents).toBe(5);
    expect(logged()).toBe("");
  });
});
