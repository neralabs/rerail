import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { randomUUID } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { loadNativeBinding, LOCKS_DIR, SqliteAuditControlStore, type AdapterEvent, type CodeRuleAuditor } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
import { SqliteSessionStore } from "../../../src/runtime/session-store.js";
import { testEmbedder } from "../../support/hook-events.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { startTestRepository } from "../../support/started-repository.js";

// A daemon that died leaves its owner rows naming its pid, which the system can give the next daemon (a container, a
// restart) or another process (Windows reuses pids soon). The next daemon's start must still end the sessions it left,
// and never one of a daemon that still runs.

/** What happens right after a sweep has judged a lock (`ownerLockFreed`), once each, in order: a test's stand-in for a death. */
const afterJudged = vi.hoisted(() => [] as Array<() => void>);
vi.mock("../../../src/engine/audit/owner-lock.js", async importOriginal => {
  const actual = await importOriginal<{ ownerLockFreed: (databasePath: string, lockId: string) => boolean }>();
  return {
    ...actual,
    ownerLockFreed: (databasePath: string, lockId: string) => {
      const freed = actual.ownerLockFreed(databasePath, lockId);
      afterJudged.shift()?.();
      return freed;
    },
  };
});

const CHILD = fileURLToPath(new URL("../../support/processes/engine-service-process.ts", import.meta.url));
const IDLE_MS = 60_000;
const DAY = 24 * 60 * 60_000;

const roots: string[] = [];
const services: EngineService[] = [];
const children: ChildProcess[] = [];

/** Kills `child` unless it has exited, and waits for its exit: a folder it runs in cannot be removed before. */
function ended(child: ChildProcess): Promise<void> {
  if (child.exitCode !== null || child.signalCode !== null) return Promise.resolve();
  const exited = new Promise<void>(resolve => child.once("exit", () => resolve()));
  child.kill("SIGKILL");
  return exited;
}

afterEach(async () => {
  afterJudged.splice(0);
  await Promise.all(children.splice(0).map(ended));
  await Promise.allSettled(services.splice(0).map(service => service.close()));
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

const quietAuditor = (): CodeRuleAuditor => ({
  openWorkspace: async () => undefined,
  acceptsPath: () => true,
  auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
  close: async () => undefined,
});

let sockets = 0;
function socketIn(root: string): string {
  sockets += 1;
  return process.platform === "win32" ? `\\\\.\\pipe\\rerail-dead-daemon-${process.pid}-${Date.now()}-${sockets}` : join(root, `s${sockets}.sock`);
}

/** A started checkout and the database its sessions are recorded in. */
async function records(): Promise<{ root: string; databasePath: string; checkout: string }> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-dead-daemon-")));
  roots.push(root);
  const databasePath = join(root, "rerail.sqlite");
  const checkout = join(root, "repo");
  await mkdir(checkout);
  startTestRepository(checkout, databasePath);
  return { root, databasePath, checkout };
}

/** An engine service in this process over the same records, with `now` as its clock. */
async function service(root: string, databasePath: string, now: () => number): Promise<EngineService> {
  const started = await startEngineService({
    socketPath: socketIn(root), storageRoot: join(root, "state"), databasePath, embedder: testEmbedder, idleSessionMs: IDLE_MS,
    idleRuntimeMs: 0, now, createAuditor: quietAuditor,
  });
  services.push(started);
  return started;
}

/** The next message of `child` that `wanted` accepts, once `send` is sent when given. */
function answer(child: ChildProcess, wanted: (message: Record<string, unknown>) => boolean, send?: object): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("the daemon process did not answer within 60 s")), 60_000);
    const listen = (message: Record<string, unknown>) => {
      if (!wanted(message)) return;
      clearTimeout(timer);
      child.off("message", listen);
      resolve(message);
    };
    child.on("message", listen);
    if (send !== undefined) child.send(send);
  });
}

/** A daemon's engine service in a process of its own (`engine-service-process.ts`) over the same records. */
async function daemonProcess(root: string, databasePath: string) {
  const child = spawn(process.execPath, ["--import", "tsx", CHILD, socketIn(root), join(root, "state"), databasePath, String(IDLE_MS)], {
    cwd: process.cwd(), stdio: ["ignore", "ignore", "pipe", "ipc"],
  });
  children.push(child);
  let stderr = "";
  child.stderr!.on("data", chunk => { stderr += String(chunk); });
  const exited = new Promise<void>(resolve => child.once("exit", () => resolve()));
  const ready = await answer(child, message => message.ready === true).catch((error: unknown) => {
    throw new Error(`${String(error)}; its stderr: ${stderr.slice(0, 2_000)}`);
  });
  return {
    send: (event: AdapterEvent) => ingestEngineEvent({ socketPath: String(ready.socketPath) }, event),
    /** Moves its clock `offset` past the wall clock and runs its idle close; answers its clock after it. */
    idleClose: async (offset: number) => Number((await answer(child, message => message.done === true, { offset, sweep: true })).now),
    /** Ends it as a machine restart or a kill does: none of its own clean-up runs. */
    kill: async () => {
      child.kill("SIGKILL");
      await exited;
    },
  };
}

/** A process that runs until the test ends, started now: one the system gave a dead daemon's pid. */
async function runningProcess(): Promise<number> {
  const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1 << 30)"], { stdio: "ignore" });
  children.push(child);
  await new Promise<void>((resolve, reject) => {
    child.once("spawn", () => resolve());
    child.once("error", reject);
  });
  return child.pid!;
}

function events(cwd: string) {
  const session = (sessionId: string) => ({ host: "claude-code" as const, sessionId, cwd });
  return {
    start: (sessionId: string): AdapterEvent => ({ kind: "session-start", eventId: `${sessionId}:start`, session: session(sessionId) }),
    prompt: (sessionId: string, text: string): AdapterEvent => ({ kind: "user-prompt", eventId: `${sessionId}:${text}`, sessionId, session: session(sessionId), text }),
  };
}

/** The session's record, its owner row and when it was last heard from, read through a connection of its own. */
function recorded(databasePath: string, sessionId: string) {
  const database = new Database(databasePath, { readonly: true });
  try {
    return database.prepare(`SELECT s.ended_at AS endedAt,
      (SELECT observed_at FROM events e WHERE e.session_id = s.session_id ORDER BY sequence DESC LIMIT 1) AS heardAt
      FROM sessions s WHERE s.session_id = ?`).get(sessionId) as { endedAt: number | null; heardAt: number };
  } finally {
    database.close();
  }
}

/** Whether the session reads as live, owned by a runtime that runs, as a scan request or the dashboard reads it. */
function live(databasePath: string, sessionId: string): boolean {
  const controls = new SqliteAuditControlStore(databasePath);
  try {
    return controls.getSettings(sessionId).active;
  } finally {
    controls.close();
  }
}

/** A scan asked for the session, as `rerail scan --session` asks the daemon for one. */
function askScan(databasePath: string, sessionId: string): { id: string; status: string; error: string | null } {
  const controls = new SqliteAuditControlStore(databasePath);
  try {
    return controls.requestAudit(sessionId, { scope: "repository" });
  } finally {
    controls.close();
  }
}

/** What became of the scan `id` asked for the session: its status and error. */
function scanOf(databasePath: string, sessionId: string, id: string): { status: string; error: string | null } | undefined {
  const controls = new SqliteAuditControlStore(databasePath);
  try {
    const request = controls.listAuditRequests(sessionId).find(item => item.id === id);
    return request === undefined ? undefined : { status: request.status, error: request.error };
  } finally {
    controls.close();
  }
}

/** How a scan queued for a session of a daemon that died ends once a later daemon ends that session. */
const INTERRUPTED = { status: "interrupted", error: "Owning runtime exited before scan could complete" };

/** Rewrites the owner row of `sessionId`, through a connection of its own. */
function setOwner(databasePath: string, sessionId: string, row: { processId: number; token?: string; updatedAt?: number }): void {
  const database = new Database(databasePath);
  try {
    const { changes } = database.prepare(`UPDATE policy_session_owners SET process_id = ?, token = coalesce(?, token),
      updated_at = coalesce(?, updated_at) WHERE session_id = ?`).run(row.processId, row.token ?? null, row.updatedAt ?? null, sessionId);
    if (changes !== 1) throw new Error(`no owner row of ${sessionId}`);
  } finally {
    database.close();
  }
}

/**
 * Opens `session`'s record and claims it from this process: under the lock `lockId` as an engine service's runtime
 * does, or without one as a pi or OMP session's runtime does.
 */
function claimHere(databasePath: string, session: { host: "claude-code"; sessionId: string; cwd: string }, lockId?: string): void {
  const sessions = new SqliteSessionStore(databasePath);
  const controls = new SqliteAuditControlStore(databasePath);
  try {
    sessions.openSession(session);
    controls.registerOwner(session.sessionId, "next-event", lockId);
  } finally {
    controls.close();
    sessions.close();
  }
}

describe("the next daemon's start after a daemon that died", () => {
  const holders = [
    ["this process", async () => process.pid],
    ["another process, started after that daemon died", runningProcess],
  ] as const;
  // The idle close ended the quiet session's record and released its owner before its daemon died. A session kept open
  // while quiet must end the same way at the next start, whatever process holds its daemon's pid by then.
  for (const [holder, pidHolder] of holders) {
    it(`ends a quiet session its daemon kept open, as its idle close did, when ${holder} holds that daemon's pid`, { timeout: 120_000 }, async () => {
      const { root, databasePath, checkout } = await records();
      const { start, prompt } = events(checkout);
      const daemon = await daemonProcess(root, databasePath);
      await daemon.send(start("q"));
      await daemon.send(prompt("q", "last"));
      const closedAt = await daemon.idleClose(IDLE_MS + 1_000);
      // Asked while the daemon held the session: it waits for the session's next open, which never comes there.
      const queued = askScan(databasePath, "q");
      expect(queued.status).toBe("queued");
      await daemon.kill();
      setOwner(databasePath, "q", { processId: await pidHolder() });
      const next = await service(root, databasePath, () => Date.now() + 3 * DAY);
      const { endedAt, heardAt } = recorded(databasePath, "q");
      expect(endedAt, "the record ended").not.toBeNull();
      expect(endedAt).toBeGreaterThanOrEqual(heardAt + IDLE_MS);
      expect(endedAt).toBeLessThanOrEqual(closedAt);
      expect(live(databasePath, "q")).toBe(false);
      expect(scanOf(databasePath, "q", queued.id)).toEqual(INTERRUPTED);
      // A scan asked for it is refused at once, rather than queued for a daemon that is gone.
      expect(askScan(databasePath, "q")).toMatchObject({ status: "failed", error: "Session is inactive" });
      // Its next event opens it again here, rather than finding it owned by a runtime that runs.
      await ingestEngineEvent({ socketPath: next.socketPath }, prompt("q", "again"));
      expect([recorded(databasePath, "q").endedAt, live(databasePath, "q")]).toEqual([null, true]);
    });
  }

  // A second daemon on the same records (a checkout's build beside the installed one, on Windows) and the services of
  // this process run on: the start never ends what they hold, quiet or at work.
  it("never ends a session of a daemon that still runs, nor one a service of this process holds", { timeout: 120_000 }, async () => {
    const { root, databasePath, checkout } = await records();
    const { start, prompt } = events(checkout);
    const daemon = await daemonProcess(root, databasePath);
    await daemon.send(start("quiet"));
    await daemon.send(prompt("quiet", "last"));
    await daemon.idleClose(IDLE_MS + 1_000);
    await daemon.send(start("at-work"));
    const here = await service(root, databasePath, Date.now);
    await ingestEngineEvent({ socketPath: here.socketPath }, start("here"));
    await service(root, databasePath, () => Date.now() + 3 * DAY);
    for (const sessionId of ["quiet", "at-work", "here"]) {
      expect([recorded(databasePath, sessionId).endedAt, live(databasePath, sessionId)], sessionId).toEqual([null, true]);
    }
  });

  // An owner row written under no lock (by an earlier version's daemon, a pi or OMP session's runtime, or a runtime that
  // could take none) names its writer by pid alone. One naming this process is its own only when this process made that
  // claim: the wall clock can step back after the start (NTP, a virtual machine resumed), so a row's time cannot tell a
  // claim made since from one a process that had the pid before made.
  it("ends a session whose owner row names this process under no lock only when this process did not make that claim", { timeout: 60_000 }, async () => {
    const { root, databasePath, checkout } = await records();
    const { start } = events(checkout);
    const first = await service(root, databasePath, Date.now);
    await ingestEngineEvent({ socketPath: first.socketPath }, start("earlier"));
    await ingestEngineEvent({ socketPath: first.socketPath }, start("stepped"));
    // Rows of a process that had this pid before: written before this process started, and after it by a clock that
    // has stepped back since.
    setOwner(databasePath, "earlier", { processId: process.pid, token: randomUUID(), updatedAt: Math.floor(performance.timeOrigin) - 60_000 });
    setOwner(databasePath, "stepped", { processId: process.pid, token: randomUUID(), updatedAt: Date.now() });
    // A claim this process made under no lock after its clock stepped back: its time reads as before the start.
    claimHere(databasePath, { host: "claude-code", sessionId: "mine", cwd: checkout });
    setOwner(databasePath, "mine", { processId: process.pid, updatedAt: Math.floor(performance.timeOrigin) - 60_000 });
    await service(root, databasePath, Date.now);
    for (const sessionId of ["earlier", "stepped"]) {
      expect([recorded(databasePath, sessionId).endedAt === null, live(databasePath, sessionId)], sessionId).toEqual([false, false]);
    }
    expect([recorded(databasePath, "mine").endedAt, live(databasePath, "mine")]).toEqual([null, true]);
  });
});

describe("a daemon that runs on after another on the same records died", () => {
  // Nothing starts after the death: the other daemon can run for days, and its idle sweep ends what the dead one left.
  it("ends the dead daemon's quiet session at its next idle sweep, as that daemon's idle close did, whatever holds its pid", { timeout: 120_000 }, async () => {
    const { root, databasePath, checkout } = await records();
    const { start, prompt } = events(checkout);
    const daemon = await daemonProcess(root, databasePath);
    await daemon.send(start("q"));
    await daemon.send(prompt("q", "last"));
    const closedAt = await daemon.idleClose(IDLE_MS + 1_000);
    const other = await service(root, databasePath, () => Date.now() + 3 * DAY);
    const queued = askScan(databasePath, "q");
    expect(queued.status).toBe("queued");
    await daemon.kill();
    setOwner(databasePath, "q", { processId: await runningProcess() });
    await other.closeIdleSessions();
    const { endedAt, heardAt } = recorded(databasePath, "q");
    expect(endedAt, "the record ended").not.toBeNull();
    expect(endedAt).toBeGreaterThanOrEqual(heardAt + IDLE_MS);
    expect(endedAt).toBeLessThanOrEqual(closedAt);
    expect(live(databasePath, "q")).toBe(false);
    expect(scanOf(databasePath, "q", queued.id)).toEqual(INTERRUPTED);
  });

  // A runtime opens a session's record again, then claims it in a second write. Another daemon's sweep can end the record
  // in between, judging the row the claim replaces, of a writer that ended.
  it("keeps open a session whose record a sweep ended between its open and its owner's claim", async () => {
    const { root, databasePath } = await records();
    const sessions = new SqliteSessionStore(databasePath);
    const controls = new SqliteAuditControlStore(databasePath);
    try {
      const session = { host: "claude-code" as const, sessionId: "x", cwd: root };
      sessions.openSession(session);
      controls.registerOwner("x", "next-event");
      setOwner(databasePath, "x", { processId: spawnSync(process.execPath, ["-e", ""]).pid! });
      sessions.closeSession("x");
      sessions.openSession(session);
      expect(controls.endSessionsOfEndedOwners(IDLE_MS, Date.now())).toBe(1);
      controls.registerOwner("x", "next-event");
      expect([recorded(databasePath, "x").endedAt, live(databasePath, "x")]).toEqual([null, true]);
    } finally {
      controls.close();
      sessions.close();
    }
  });
});

describe("the lock file of a daemon that died", () => {
  // A daemon that dies just after another's sweep judged its lock held leaves a lock the same sweep then finds free.
  // Removed then, while that daemon's sessions' rows still named it, the lock could no longer be found, and a sweep
  // takes a lock with no file for one that may still run: those sessions read as live and owned for good while a
  // process had the dead daemon's pid.
  it("stays while an open session's owner row names it, so the next sweep ends that session", { timeout: 60_000 }, async () => {
    const { root, databasePath } = await records();
    const lockId = randomUUID();
    const owners = join(root, LOCKS_DIR, "owners");
    const lockFile = join(owners, `${lockId}.lock`);
    await mkdir(owners, { recursive: true });
    // The other daemon's lock, held as its engine service holds it. Its session's row names this process: a live pid,
    // as a reused one is.
    const held = loadNativeBinding().tryLockExclusive(lockFile);
    if (held === null) throw new Error("the other daemon's lock was not taken");
    claimHere(databasePath, { host: "claude-code", sessionId: "d", cwd: root }, lockId);
    afterJudged.push(() => held.release());
    const next = await service(root, databasePath, Date.now);
    expect(existsSync(lockFile), "the dead daemon's lock file").toBe(true);
    await next.closeIdleSessions();
    expect([recorded(databasePath, "d").endedAt === null, live(databasePath, "d")]).toEqual([false, false]);
  });
});
