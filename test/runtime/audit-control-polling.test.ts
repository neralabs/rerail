import { realpathSync } from "node:fs";
import { mkdir, realpath, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import Database from "better-sqlite3";
import { eq } from "drizzle-orm";
import { afterEach, describe, expect, it, vi } from "vitest";
import { auditSettings, audits, codeRuleSchema, NativeCodeRuleEnforcer, openRerailDatabase, recordCheckout, SqliteAuditControlStore } from "../../src/engine/index.js";
import { RerailRuntime, type RerailRuntimeOptions } from "../../src/runtime/runtime.js";
import { SqliteSessionStore } from "../../src/runtime/session-store.js";
import { temporaryDirectories, testEmbedder } from "../support/hook-events.js";
import { testCodeRuleCatalog } from "../support/repository-rules.js";

const temporaryDirectory = temporaryDirectories("rerail-control-polling-");

afterEach(() => {
  vi.restoreAllMocks();
});

function catalog() {
  return testCodeRuleCatalog([codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
  })]);
}

/**
 * A runtime holding `sessions` open; `background` gives it a way to deliver steers between events, and `createAuditor`
 * makes its sessions' evaluators (the needle rule's by default).
 */
async function idleRuntime(
  sessions: readonly string[],
  { background = false, createAuditor = options => new NativeCodeRuleEnforcer(catalog(), options) }: {
    background?: boolean;
    createAuditor?: RerailRuntimeOptions["createAuditor"];
  } = {},
) {
  const root = await realpath(await temporaryDirectory());
  const databasePath = join(root, "rerail.sqlite");
  const runtime = new RerailRuntime({
    storageRoot: join(root, "sessions"), databasePath, workspaceRoot: root, autostart: false, embedder: testEmbedder, createAuditor,
    ...(background ? { deliverBackground: () => undefined } : {}),
  });
  for (const sessionId of sessions) {
    await runtime.ingest({ kind: "session-start", eventId: `start-${sessionId}`, session: { host: "claude-code", sessionId, cwd: root } });
    await runtime.codeRulesOpened(sessionId);
  }
  return { runtime, databasePath, root };
}

/**
 * Removes `sessionId`'s row, with every row that names it, in one commit of another connection: what a database put
 * back from a copy taken before the session began, or another program, leaves under a running session.
 */
function removeSessionRow(databasePath: string, sessionId: string): void {
  const database = new Database(databasePath);
  try {
    database.pragma("busy_timeout = 5000");
    database.transaction(() => {
      for (const table of ["events", "work_links", "steer_feedback", "audits", "audit_requests", "repository_audits", "policy_session_owners", "sessions"])
        database.prepare(`DELETE FROM ${table} WHERE session_id = ?`).run(sessionId);
    })();
  } finally {
    database.close();
  }
}

/** The session's row as the database holds it, and how many of its recorded events have the id `eventId`. */
function recorded(databasePath: string, sessionId: string, eventId: string): { session: unknown; events: unknown } {
  const database = new Database(databasePath, { readonly: true });
  try {
    return {
      session: database.prepare("SELECT ended_at AS endedAt FROM sessions WHERE session_id = ?").get(sessionId),
      events: database.prepare("SELECT count(*) AS count FROM events WHERE session_id = ? AND event_id = ?").get(sessionId, eventId),
    };
  } finally {
    database.close();
  }
}

/** The session's owner row as the database holds it, undefined when it has none. */
function ownerRow(databasePath: string, sessionId: string): unknown {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare("SELECT delivery, active, token FROM policy_session_owners WHERE session_id = ?").get(sessionId);
  } finally {
    database.close();
  }
}

/** A live claim on `sessionId` with the token `token`, as another runtime of this process writes one. */
function claimElsewhere(databasePath: string, sessionId: string, token: string): void {
  const database = new Database(databasePath);
  try {
    database.pragma("busy_timeout = 5000");
    database.prepare("INSERT INTO policy_session_owners (session_id, delivery, active, process_id, token, updated_at) VALUES (?, 'next-event', 1, ?, ?, 1)")
      .run(sessionId, process.pid, token);
  } finally {
    database.close();
  }
}

const CLAIM_REFUSED = "this claim write is refused";

/** Runs `statement` through a connection of its own, as another program would. */
function runElsewhere(databasePath: string, statement: string): void {
  const database = new Database(databasePath);
  try {
    database.pragma("busy_timeout = 5000");
    database.exec(statement);
  } finally {
    database.close();
  }
}

/**
 * Makes the database refuse every write of a session's claim, as a write that meets SQLITE_BUSY past its wait or an
 * I/O error fails, until the function it returns lets them through again.
 */
function refuseClaimWrites(databasePath: string): () => void {
  runElsewhere(databasePath, `CREATE TRIGGER refuse_claim_writes BEFORE INSERT ON policy_session_owners BEGIN SELECT RAISE(ABORT, '${CLAIM_REFUSED}'); END`);
  return () => runElsewhere(databasePath, "DROP TRIGGER refuse_claim_writes");
}

describe("The runtime's audit control polling", () => {
  // The daemon holds every open session of the machine; reading each one's settings and queue four times a second
  // kept an idle daemon at about 1.5% of a core with three sessions open, and more with each session.
  it("reads no idle session's settings or audit requests until another connection commits", async () => {
    const { runtime, databasePath } = await idleRuntime(["a", "b", "c"]);
    try {
      // Every session is read once after it opens.
      await sleep(600);
      const claims = vi.spyOn(SqliteAuditControlStore.prototype, "claim");
      const settings = vi.spyOn(SqliteAuditControlStore.prototype, "getWorkspaceSettings");
      await sleep(1_500);
      expect(claims.mock.calls.length + settings.mock.calls.length).toBe(0);

      // `rerail scan --session` queues through its own connection; the next tick claims it.
      const other = new SqliteAuditControlStore(databasePath);
      try {
        const queued = other.requestAudit("b", { scope: "repository" });
        await vi.waitFor(() => {
          expect(other.listAuditRequests("b").find(request => request.id === queued.id)?.status).not.toBe("queued");
        }, { timeout: 3_000, interval: 50 });
        expect(claims.mock.calls.map(([sessionId]) => sessionId)).toContain("b");
      } finally {
        other.close();
      }
    } finally {
      await runtime.dispose();
    }
  }, 20_000);

  it("claims a second request queued behind a running one once the first finishes, with no other commit", async () => {
    const { runtime, databasePath } = await idleRuntime(["a"]);
    const other = new SqliteAuditControlStore(databasePath);
    try {
      await sleep(600);
      // Both are visible to one tick; that tick claims only the oldest.
      const first = other.requestAudit("a", { scope: "repository" });
      const second = other.requestAudit("a", { scope: "repository" });
      await vi.waitFor(() => {
        const statuses = other.listAuditRequests("a").filter(request => [first.id, second.id].includes(request.id)).map(request => request.status);
        expect(statuses.every(status => status !== "queued" && status !== "running")).toBe(true);
      }, { timeout: 10_000, interval: 50 });
    } finally {
      other.close();
      await runtime.dispose();
    }
  }, 20_000);

  // The engine service keeps a runtime no session uses open for its checkout's next session. Its control timer read
  // the database's version four times a second all the same, for nothing: it looks only for open sessions' changes.
  it("reads nothing while no session is open in it, and looks again from the next session's start", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const versions = vi.spyOn(SqliteAuditControlStore.prototype, "dataVersion");
    const { runtime, root } = await idleRuntime([]);
    try {
      vi.advanceTimersByTime(1_000);
      expect(versions).not.toHaveBeenCalled();
      const session = { host: "claude-code" as const, sessionId: "a", cwd: root };
      await runtime.ingest({ kind: "session-start", eventId: "start-a", session });
      versions.mockClear();
      vi.advanceTimersByTime(1_000);
      expect(versions).toHaveBeenCalled();
      await runtime.ingest({ kind: "session-end", eventId: "end-a", sessionId: "a", session });
      versions.mockClear();
      vi.advanceTimersByTime(1_000);
      expect(versions).not.toHaveBeenCalled();
    } finally {
      await runtime.dispose();
      vi.useRealTimers();
    }
  });

  it("reads nothing once the only session's open has failed", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const versions = vi.spyOn(SqliteAuditControlStore.prototype, "dataVersion");
    const { runtime, root } = await idleRuntime([], { createAuditor: () => {
      throw new Error("the evaluator did not open");
    } });
    try {
      const session = { host: "claude-code" as const, sessionId: "a", cwd: root };
      await expect(runtime.ingest({ kind: "session-start", eventId: "start-a", session })).rejects.toThrow("the evaluator did not open");
      versions.mockClear();
      vi.advanceTimersByTime(1_000);
      expect(versions).not.toHaveBeenCalled();
    } finally {
      await runtime.dispose();
      vi.useRealTimers();
    }
  });

  // A tick claims a queued scan for an open session: one that came while the runtime closed its sessions one by one
  // would start a scan in a session being closed.
  it("starts no control timer again while it closes its sessions", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    const held = Promise.withResolvers<void>();
    let made = 0;
    const { runtime } = await idleRuntime(["a", "b"], { createAuditor: (options) => {
      made += 1;
      const enforcer = new NativeCodeRuleEnforcer(catalog(), options);
      if (made !== 2) return enforcer;
      const close = enforcer.close.bind(enforcer);
      return Object.assign(enforcer, { close: async () => {
        await held.promise;
        await close();
      } });
    } });
    const closed = vi.spyOn(SqliteSessionStore.prototype, "closeSession");
    const versions = vi.spyOn(SqliteAuditControlStore.prototype, "dataVersion");
    const disposing = runtime.dispose();
    try {
      await vi.waitFor(() => expect(closed.mock.calls.map(([sessionId]) => sessionId)).toContain("a"));
      versions.mockClear();
      vi.advanceTimersByTime(1_000);
      expect(versions).not.toHaveBeenCalled();
    } finally {
      held.resolve();
      await disposing;
      vi.useRealTimers();
    }
  });

  // Another program can remove a running session's row, as a database put back from a copy taken before the session
  // began does. The tick read each session's settings through its row, and the throw from its timer ended the daemon
  // with every session it held.
  it("reads the other sessions' controls on once a running session's row is removed", async () => {
    const { runtime, databasePath } = await idleRuntime(["gone", "kept"]);
    const other = new SqliteAuditControlStore(databasePath);
    try {
      removeSessionRow(databasePath, "gone");
      const queued = other.requestAudit("kept", { scope: "repository" });
      await vi.waitFor(() => {
        expect(other.listAuditRequests("kept").find(request => request.id === queued.id)?.status).not.toBe("queued");
      }, { timeout: 3_000, interval: 50 });
    } finally {
      other.close();
      await runtime.dispose();
    }
  }, 20_000);

  it("answers and records a running session's next event once its row is removed, and its row is back", async () => {
    const { runtime, databasePath } = await idleRuntime(["gone"]);
    try {
      removeSessionRow(databasePath, "gone");
      await runtime.ingest({ kind: "user-prompt", eventId: "after-removal", sessionId: "gone", text: "go on" });
      expect(recorded(databasePath, "gone", "after-removal")).toEqual({ session: { endedAt: null }, events: { count: 1 } });
    } finally {
      await runtime.dispose();
    }
  }, 20_000);

  // The session's claim went with its rows: a scan asked for in it (`rerail scan --session`) needs a live owner, and a
  // daemon started again would claim the session when it opened it.
  it.each([false, true])("claims a scan asked for in a running session once its row is removed and its next event has put the row back (steers between events: %s)", async background => {
    const { runtime, databasePath } = await idleRuntime(["gone"], { background });
    const other = new SqliteAuditControlStore(databasePath);
    try {
      const claimed = ownerRow(databasePath, "gone");
      expect(claimed).toMatchObject({ delivery: background ? "background" : "next-event", active: 1 });
      removeSessionRow(databasePath, "gone");
      await runtime.ingest({ kind: "user-prompt", eventId: "after-removal", sessionId: "gone", text: "go on" });
      expect(ownerRow(databasePath, "gone")).toEqual(claimed);
      const queued = other.requestAudit("gone", { scope: "repository" });
      expect({ status: queued.status, error: queued.error }).toEqual({ status: "queued", error: null });
      await vi.waitFor(() => {
        expect(other.listAuditRequests("gone").find(request => request.id === queued.id)?.status).not.toBe("queued");
      }, { timeout: 3_000, interval: 50 });
    } finally {
      other.close();
      await runtime.dispose();
    }
  }, 20_000);

  it("leaves a session another runtime claimed since to it when the session's next event puts its row back", async () => {
    const { runtime, databasePath } = await idleRuntime(["gone"]);
    try {
      removeSessionRow(databasePath, "gone");
      claimElsewhere(databasePath, "gone", "another-runtime");
      await runtime.ingest({ kind: "user-prompt", eventId: "after-removal", sessionId: "gone", text: "go on" });
      expect(ownerRow(databasePath, "gone")).toEqual({ delivery: "next-event", active: 1, token: "another-runtime" });
    } finally {
      await runtime.dispose();
    }
  }, 20_000);

  // A claim written again for a session that has let go of it would hold every scan asked for there queued for good.
  it("writes no claim again for a session that is closing when a write of its close puts its row back", async () => {
    const { runtime, databasePath } = await idleRuntime(["gone"]);
    try {
      removeSessionRow(databasePath, "gone");
      const closing = runtime.closeSession("gone");
      runtime.recordSteer("gone", "while-closing");
      await closing;
      expect(ownerRow(databasePath, "gone")).toBeUndefined();
    } finally {
      await runtime.dispose();
    }
  }, 20_000);

  // The claim is written after the event and the row are saved: a hook told that its event failed would report a
  // failed write for an event Rerail holds.
  it("records a session's event that put its row back when the claim written with it fails, and says why", async () => {
    const { runtime, databasePath } = await idleRuntime(["gone"]);
    try {
      removeSessionRow(databasePath, "gone");
      const allowClaimWrites = refuseClaimWrites(databasePath);
      const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
      await runtime.ingest({ kind: "user-prompt", eventId: "after-removal", sessionId: "gone", text: "go on" });
      allowClaimWrites();
      expect(recorded(databasePath, "gone", "after-removal")).toEqual({ session: { endedAt: null }, events: { count: 1 } });
      expect(stderr.mock.calls.map(([text]) => String(text)).filter(text => text.includes(CLAIM_REFUSED)))
        .toEqual([expect.stringMatching(/^rerail: .*gone/)]);
    } finally {
      await runtime.dispose();
    }
  }, 20_000);

  // Once the row is back no write puts it back again, so a claim that failed there would stay away until the session
  // opened again, and every scan asked for in it would fail.
  it("writes at the session's next event the claim a failed write left owed, and a scan asked for there completes", async () => {
    const { runtime, databasePath } = await idleRuntime(["gone"]);
    const other = new SqliteAuditControlStore(databasePath);
    try {
      const claimed = ownerRow(databasePath, "gone");
      removeSessionRow(databasePath, "gone");
      const allowClaimWrites = refuseClaimWrites(databasePath);
      vi.spyOn(process.stderr, "write").mockImplementation(() => true);
      await runtime.ingest({ kind: "user-prompt", eventId: "row-put-back", sessionId: "gone", text: "go on" });
      allowClaimWrites();
      expect(ownerRow(databasePath, "gone")).toBeUndefined();
      await runtime.ingest({ kind: "user-prompt", eventId: "claim-owed", sessionId: "gone", text: "go on" });
      expect(ownerRow(databasePath, "gone")).toEqual(claimed);
      // Owed no more: the session's events write no claim, as before the row went.
      const claimWrites = vi.spyOn(SqliteAuditControlStore.prototype, "claimAgain");
      await runtime.ingest({ kind: "user-prompt", eventId: "claimed", sessionId: "gone", text: "go on" });
      expect(claimWrites).not.toHaveBeenCalled();
      const queued = other.requestAudit("gone", { scope: "repository" });
      expect({ status: queued.status, error: queued.error }).toEqual({ status: "queued", error: null });
      await vi.waitFor(() => {
        expect(other.listAuditRequests("gone").find(request => request.id === queued.id)?.status).not.toBe("queued");
      }, { timeout: 3_000, interval: 50 });
    } finally {
      other.close();
      await runtime.dispose();
    }
  }, 20_000);
});

describe.runIf(process.platform === "win32")("A session in a Windows folder outside Git spelt otherwise than the disk stores it", () => {
  /**
   * Log mode saved by Rerail 0.1.4 for a folder `App` reached as VS Code or a typed `cd` can spell it (`c:\…\app`):
   * under the JS realpath, which keeps that spelling, beside the checkout row 0.1.4 recorded under the same key when its
   * session opened there, naming the folder's own clone and repository, else `repositoryKey`'s. Then a session there writes a
   * file that breaks a rule, through a runtime of this build, and the mode each audit of that rule was checked in;
   * `afterOpen` changes the database once the session is open, before the write.
   */
  async function breakRuleWhere014SavedLog(repositoryKey?: string, afterOpen?: (databasePath: string) => void): Promise<{ deliveries: readonly unknown[]; modes: string[] }> {
    const root = await realpath(await temporaryDirectory());
    const stored = join(root, "App");
    await mkdir(stored);
    const spelled = join(dirname(stored).replace(/^[A-Z]:/u, drive => drive.toLowerCase()), "app");
    const earlierKey = realpathSync(spelled);
    expect(earlierKey).not.toBe(stored);
    const databasePath = join(root, "rerail.sqlite");
    const database = openRerailDatabase(databasePath);
    try {
      const recorded = recordCheckout(database, spelled);
      database.insert(auditSettings).values({ workspaceRoot: earlierKey, settings: { scope: "ai-output", mode: "log" }, updatedAt: 1 }).run();
      if (repositoryKey !== undefined) database.$client.prepare("INSERT INTO repositories (repository_key, display_name, started, configured_at, created_at, updated_at) VALUES (?, 'Another', 1, 1, 1, 1)").run(repositoryKey);
      database.$client.prepare("UPDATE checkouts SET checkout_path = ?, repository_key = ?, clone_id = ? WHERE checkout_path = ?")
        .run(earlierKey, repositoryKey ?? recorded.repositoryKey, repositoryKey ?? recorded.cloneId, stored);
    } finally { database.$client.close(); }
    const runtime = new RerailRuntime({
      storageRoot: join(root, "sessions"), databasePath, workspaceRoot: stored, autostart: false, embedder: testEmbedder,
      createAuditor: options => new NativeCodeRuleEnforcer(catalog(), options),
    });
    try {
      await runtime.ingest({ kind: "session-start", eventId: "start", session: { host: "omp", sessionId: "spelled", cwd: spelled } });
      await runtime.codeRulesOpened("spelled");
      afterOpen?.(databasePath);
      await runtime.ingest({ kind: "call", eventId: "call", sessionId: "spelled", toolCallId: "w1", toolName: "write", input: { path: "created.py", content: "" } });
      await writeFile(join(stored, "created.py"), "NEEDLE = 1\n");
      const deliveries = await runtime.ingest({ kind: "result", eventId: "result", sessionId: "spelled", toolCallId: "w1", toolName: "write", output: "written" });
      const reader = openRerailDatabase(databasePath);
      try {
        const modes = reader.select({ mode: audits.mode, violations: audits.violations }).from(audits).where(eq(audits.sessionId, "spelled")).all()
          .filter(audit => audit.violations.some(violation => violation.ruleId === "test:needle")).map(audit => audit.mode);
        return { deliveries, modes };
      } finally { reader.$client.close(); }
    } finally {
      await runtime.dispose();
    }
  }

  it("checks in the Log mode Rerail 0.1.4 saved for that spelling, recording the broken rule without telling the coding agent", async () => {
    expect(await breakRuleWhere014SavedLog()).toEqual({ deliveries: [], modes: ["log"] });
  }, 20_000);

  it("checks in Steer where 0.1.4 saved Log mode under that spelling for another repository's folder, as a drive letter now leading elsewhere leaves it", async () => {
    expect((await breakRuleWhere014SavedLog("clone:another")).modes).toEqual(["steer"]);
  }, 20_000);

  // The settings are read by the folder as the session spelt it and the session's repository, the facts its row held.
  it("checks in that Log mode once another program removed the session's row", async () => {
    expect(await breakRuleWhere014SavedLog(undefined, databasePath => removeSessionRow(databasePath, "spelled"))).toEqual({ deliveries: [], modes: ["log"] });
  }, 20_000);
});
