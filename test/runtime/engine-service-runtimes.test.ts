import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import type { AdapterEvent } from "../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../src/runtime/engine-service.js";
import { testEmbedder } from "../support/hook-events.js";
import { startTestRepository } from "../support/started-repository.js";

const roots: string[] = [];
const services: EngineService[] = [];
afterEach(async () => {
  await Promise.all(services.splice(0).map(service => service.close()));
  // A session's source walk can still hold a checkout folder open for a moment after its runtime closed, which fails an
  // immediate remove on Windows (EBUSY).
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true, maxRetries: 10 })));
});

describe("the engine service's runtimes", () => {
  it("closes a checkout's runtime when its last session ends, so a daemon serving many worktrees does not keep them all", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-runtimes-"));
    roots.push(root);
    const databasePath = join(root, "rerail.sqlite");
    const [first, second] = [join(root, "first"), join(root, "second")];
    for (const checkout of [first, second]) {
      await mkdir(checkout);
      startTestRepository(checkout, databasePath);
    }
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-runtimes-${process.pid}-${Date.now()}` : join(root, "engine.sock");
    const service = await startEngineService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath,
      embedder: testEmbedder,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    services.push(service);
    const send = (event: AdapterEvent) => ingestEngineEvent({ socketPath }, event);
    const session = (sessionId: string, cwd: string) => ({ host: "claude-code" as const, sessionId, cwd });
    const start = (sessionId: string, cwd: string) => send({ kind: "session-start", eventId: `${sessionId}:start`, session: session(sessionId, cwd) });
    const end = (sessionId: string, cwd: string) => send({ kind: "session-end", eventId: `${sessionId}:end`, sessionId, session: session(sessionId, cwd) });

    await start("a", first);
    await start("b", second);
    await start("c", second);
    expect(service.openRuntimes()).toBe(2);
    await end("a", first);
    expect(service.openRuntimes()).toBe(1);
    await end("b", second);
    expect(service.openRuntimes()).toBe(1);
    await end("c", second);
    expect(service.openRuntimes()).toBe(0);
    // The checkout's next session gets a runtime of its own, as its first did.
    await start("d", first);
    await send({ kind: "user-prompt", eventId: "d:prompt", sessionId: "d", session: session("d", first), text: "hi" });
    expect(service.openRuntimes()).toBe(1);
  });

  it("closes a session that sent no end once it has gone quiet, so a killed agent's session does not stay forever", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-idle-"));
    roots.push(root);
    const databasePath = join(root, "rerail.sqlite");
    const [first, second] = [join(root, "first"), join(root, "second")];
    for (const checkout of [first, second]) {
      await mkdir(checkout);
      startTestRepository(checkout, databasePath);
    }
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-idle-${process.pid}-${Date.now()}` : join(root, "engine.sock");
    let now = 1_000_000;
    const service = await startEngineService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath,
      embedder: testEmbedder,
      idleSessionMs: 60_000,
      now: () => now,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
        close: async () => undefined,
      }),
    });
    services.push(service);
    const send = (event: AdapterEvent) => ingestEngineEvent({ socketPath }, event);
    const session = (sessionId: string, cwd: string) => ({ host: "claude-code" as const, sessionId, cwd });
    const prompt = (sessionId: string, cwd: string, text: string) =>
      send({ kind: "user-prompt", eventId: `${sessionId}:${text}`, sessionId, session: session(sessionId, cwd), text });
    const ended = (sessionId: string) => endedAt(databasePath, sessionId);

    await send({ kind: "session-start", eventId: "a:start", session: session("a", first) });
    await send({ kind: "session-start", eventId: "b:start", session: session("b", second) });
    now += 50_000;
    await prompt("b", second, "still here");
    now += 20_000;
    // `a` has been quiet for 70 s, `b` for 20 s: only `a` closes, and its checkout's runtime with it.
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    expect(ended("a")).not.toBeNull();
    expect(ended("b")).toBeNull();
    // A closed session's next event opens it again from its ledger.
    await prompt("a", first, "back");
    expect(service.openRuntimes()).toBe(2);
    expect(ended("a")).toBeNull();
  });
});

/** When the database says `sessionId` ended, read through a connection of its own; null while it runs. */
function endedAt(databasePath: string, sessionId: string): number | null {
  const database = new Database(databasePath, { readonly: true });
  try {
    const row = database.prepare("SELECT ended_at AS endedAt FROM sessions WHERE session_id = ?").get(sessionId) as { endedAt: number | null } | undefined;
    if (row === undefined) throw new Error(`no session ${sessionId}`);
    return row.endedAt;
  } finally {
    database.close();
  }
}
