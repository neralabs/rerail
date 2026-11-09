import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { AdapterEvent } from "../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../src/runtime/engine-service.js";
import { testEmbedder } from "../support/hook-events.js";
import { startTestRepository } from "../support/started-repository.js";

const roots: string[] = [];
const services: EngineService[] = [];
afterEach(async () => {
  await Promise.all(services.splice(0).map(service => service.close()));
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
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
});
