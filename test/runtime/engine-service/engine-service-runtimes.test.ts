import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { runInNewContext } from "node:vm";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CheckoutCatalogs, codeRuleSchema, type AdapterEvent } from "../../../src/engine/index.js";
import { ingestEngineEvent, startEngineService, type EngineService } from "../../../src/runtime/engine-service.js";
import { RerailRuntime } from "../../../src/runtime/runtime.js";
import { testEmbedder } from "../../support/hook-events.js";
import { setUpTestRepository, type TestRepositoryRules } from "../../support/repository-rules.js";
import { startTestRepository } from "../../support/started-repository.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

// The service collects through `runInNewContext("gc")` once a runtime closed; one test makes that collection fail.
vi.mock("node:vm", { spy: true });

const roots: string[] = [];
const services: EngineService[] = [];
afterEach(async () => {
  await Promise.all(services.splice(0).map(service => service.close()));
  // A session's source walk can still hold a checkout folder open for a moment after its runtime closed, which fails an
  // immediate remove on Windows (EBUSY).
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/**
 * An engine service over two started checkouts, `first` (a Git repository adding `rules`) and `second`, whose
 * runtimes stay `idleRuntimeMs` unused.
 */
async function twoCheckouts(idleRuntimeMs: number, rules?: TestRepositoryRules) {
  const root = await mkdtemp(join(tmpdir(), "rerail-runtimes-"));
  roots.push(root);
  const databasePath = join(root, "rerail.sqlite");
  const [first, second] = [join(root, "first"), join(root, "second")];
  for (const checkout of [first, second]) await mkdir(checkout);
  if (rules === undefined) startTestRepository(first, databasePath);
  else {
    execFileSync("git", ["init", "--quiet"], { cwd: first });
    setUpTestRepository(first, rules, databasePath);
  }
  startTestRepository(second, databasePath);
  const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-runtimes-${process.pid}-${Date.now()}` : join(root, "engine.sock");
  const service = await startEngineService({
    socketPath,
    storageRoot: join(root, "state"),
    databasePath,
    embedder: testEmbedder,
    idleRuntimeMs,
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
  return { service, databasePath, first, second, start, end, send, session };
}

describe("the engine service's runtimes", () => {
  it("closes a checkout's runtime when its last session ends, so a daemon serving many worktrees does not keep them all", async () => {
    const { service, first, second, start, end, send, session } = await twoCheckouts(0);

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

  // A runtime holds its checkout's compiled rules, 12 s of CPU to compile for the five packs a TypeScript repository
  // adds: a session started soon after the last one ended reuses it, and the runtime closes once it was unused a while.
  it("keeps a checkout's runtime until no session has used it for the idle time, so a session that comes back soon reuses it", async () => {
    // Only the idle timer is faked: the runtime's own timers are intervals, and the HTTP client's are the socket's.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const { service, first, start, end } = await twoCheckouts(60_000);

      await start("a", first);
      await end("a", first);
      vi.advanceTimersByTime(59_999);
      expect(service.openRuntimes()).toBe(1);
      await start("b", first);
      vi.advanceTimersByTime(5 * 60_000);
      expect(service.openRuntimes()).toBe(1);
      // The idle time counts from the last session's end.
      await end("b", first);
      vi.advanceTimersByTime(59_999);
      expect(service.openRuntimes()).toBe(1);
      vi.advanceTimersByTime(1);
      expect(service.openRuntimes()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  // A compiled rule's program is native memory V8 does not count, 512 MB for the Node pack's builtins rule, and a daemon
  // with no session open allocates too little to start a collection: it kept a closed runtime's rules until the next
  // session came.
  it("frees a closed runtime's compiled rules without waiting for the daemon to allocate", async () => {
    const rule = codeRuleSchema.parse({
      format: "rerail.policy/v1", id: "test:freed", revision: 1, mode: "block",
      scope: { include: ["**/*.py"], exclude: [] },
      clauses: [{ id: "test:freed:check", violation: { anchor: "witness.span", code: "NO_PRINT", message: "No print" },
        forbid: { ast: { language: "python", rule: { pattern: "print($ARG)" } } } }],
    });
    const { service, databasePath, first, start, end } = await twoCheckouts(0, { library: [{ kind: "code", rule }] });

    await start("a", first);
    // The rule as this checkout's catalogs compile it: the open runtime's catalog shares the executable.
    const compiled = ((): WeakRef<object> => {
      const catalogs = new CheckoutCatalogs(databasePath);
      try {
        const [entry] = catalogs.catalog(first).snapshot().codeRules;
        if (entry === undefined) throw new Error("the library rule did not compile");
        return new WeakRef(entry.executable);
      } finally {
        catalogs.close();
      }
    })();

    await end("a", first);
    expect(service.openRuntimes()).toBe(0);
    // Nothing in this test collects: the service does, once the runtime closed.
    await vi.waitFor(() => expect(compiled.deref()).toBeUndefined(), { timeout: 5_000 });
  });

  // A runtime closing after its last session ended still has its checkout's database open: a daemon that stopped
  // before it was done would leave that work cut short.
  it("waits, as it closes, for a runtime still closing after its last session ended", async () => {
    const { service, first, start, end } = await twoCheckouts(0);
    const disposing = Promise.withResolvers<void>();
    const order: string[] = [];
    const dispose = RerailRuntime.prototype.dispose;
    const disposal = vi.spyOn(RerailRuntime.prototype, "dispose").mockImplementationOnce(async function (this: RerailRuntime) {
      await disposing.promise;
      await dispose.call(this);
      order.push("runtime closed");
    });
    try {
      await start("a", first);
      await end("a", first);
      // This test closes the service itself; until here, a failed start or end leaves it to `afterEach`.
      services.splice(services.indexOf(service), 1);
      const closing = service.close().then(() => order.push("service closed"));
      // Long enough for a close that does not wait to have returned.
      await sleep(200);
      disposing.resolve();
      await closing;
    } finally {
      disposal.mockRestore();
    }

    expect(order).toEqual(["runtime closed", "service closed"]);
  });

  // The collection after a runtime closed throws if V8 does not expose its collector. Unhandled, the throw would end the
  // daemon; swallowed, it would hide that the closed runtime's rules may still hold their memory.
  it("reports a collection that fails after a runtime closed, and still closes", async () => {
    const { service, first, start, end } = await twoCheckouts(0);
    const written: string[] = [];
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(chunk => (written.push(String(chunk)), true));
    vi.mocked(runInNewContext).mockImplementationOnce(() => {
      throw new Error("gc is not exposed");
    });
    try {
      await start("a", first);
      await end("a", first);
      // This test closes the service itself; until here, a failed start or end leaves it to `afterEach`.
      services.splice(services.indexOf(service), 1);
      await service.close();
    } finally {
      stderr.mockRestore();
      vi.mocked(runInNewContext).mockReset();
    }

    expect(service.openRuntimes()).toBe(0);
    expect(written).toContain("rerail: closing an idle runtime failed: gc is not exposed\n");
  });
});

describe("the engine service's idle close", () => {
  /** A service over two started checkouts whose sessions close after a minute of `clock` without an event. */
  async function idleService(options: Partial<Parameters<typeof startEngineService>[0]> = {}) {
    const root = await mkdtemp(join(tmpdir(), "rerail-idle-"));
    roots.push(root);
    const databasePath = join(root, "rerail.sqlite");
    const [first, second] = [join(root, "first"), join(root, "second")];
    for (const checkout of [first, second]) {
      await mkdir(checkout);
      startTestRepository(checkout, databasePath);
    }
    const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-idle-${process.pid}-${Date.now()}` : join(root, "engine.sock");
    const clock = { now: 1_000_000 };
    const service = await startEngineService({
      socketPath,
      storageRoot: join(root, "state"),
      databasePath,
      embedder: testEmbedder,
      idleSessionMs: 60_000,
      // The runtime closes when its last session does, so the closed session's runtime goes with it.
      idleRuntimeMs: 0,
      now: () => clock.now,
      createAuditor: () => ({
        openWorkspace: async () => undefined,
        acceptsPath: () => true,
        auditGeneratedEdits: async (_sessionId, batch) => ({ revision: batch.revision, disposition: "continue" as const, violations: [], failures: [] }),
        close: async () => undefined,
      }),
      ...options,
    });
    services.push(service);
    const session = (sessionId: string, cwd = first) => ({ host: "claude-code" as const, sessionId, cwd });
    const send = (event: AdapterEvent) => ingestEngineEvent({ socketPath }, event);
    return {
      first, second, clock, service, session, send,
      start: (sessionId: string, cwd = first) => send({ kind: "session-start", eventId: `${sessionId}:start`, session: session(sessionId, cwd) }),
      prompt: (sessionId: string, text: string, cwd = first) =>
        send({ kind: "user-prompt", eventId: `${sessionId}:${text}`, sessionId, session: session(sessionId, cwd), text }),
      ended: (sessionId: string) => endedAt(databasePath, sessionId),
    };
  }

  it("closes a session that sent no end once it has gone quiet, so a killed agent's session does not stay forever", async () => {
    const { second, clock, service, start, prompt, ended } = await idleService();
    await start("a");
    await start("b", second);
    clock.now += 50_000;
    await prompt("b", "still here", second);
    clock.now += 20_000;
    // `a` has been quiet for 70 s, `b` for 20 s: only `a` closes, and its checkout's runtime with it.
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    expect(ended("a")).not.toBeNull();
    expect(ended("b")).toBeNull();
    // A closed session's next event opens it again from its ledger.
    await prompt("a", "back");
    expect(service.openRuntimes()).toBe(2);
    expect(ended("a")).toBeNull();
  });

  it("keeps a quiet session open while it waits on a tool call, and closes it once the call is done", async () => {
    const { clock, service, session, send, start, ended } = await idleService();
    const tool = { sessionId: "a", session: session("a"), toolCallId: "t1", toolName: "Bash" } as const;
    await start("a");
    // A long command, or a permission prompt the user has not answered: the session sends nothing meanwhile.
    await send({ kind: "call", eventId: "a:call", ...tool, input: { command: "npm test" } });
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    expect(ended("a")).toBeNull();
    await send({ kind: "result", eventId: "a:result", ...tool, output: "ok" });
    clock.now += 60_000;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
    expect(ended("a")).not.toBeNull();
  });

  it("closes a session that waits on a tool call once it has been quiet a day, so an agent killed at a permission prompt does not keep it forever", async () => {
    const { clock, service, session, send, start, ended } = await idleService();
    const hour = 60 * 60_000;
    await start("a");
    // Claude Code closed while the call waited for the user's permission: its result and its end never come.
    await send({ kind: "call", eventId: "a:call", sessionId: "a", session: session("a"), toolCallId: "t1", toolName: "Bash", input: { command: "rm -rf build" } });
    clock.now += 23 * hour;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(1);
    expect(ended("a")).toBeNull();
    clock.now += hour;
    await service.closeIdleSessions();
    expect(service.openRuntimes()).toBe(0);
    expect(ended("a")).not.toBeNull();
  });

  it("keeps a session open whose event came in while the sweep waited behind the one before it", async () => {
    const published = Promise.withResolvers<void>();
    const publishing = Promise.withResolvers<void>();
    const { clock, service, session, ended } = await idleService({
      beforeSessionPublish: async () => {
        published.resolve();
        await publishing.promise;
      },
    });
    // The start holds the session's lane, and its prompt waits behind it.
    const starting = service.ingest({ kind: "session-start", eventId: "a:start", session: session("a") });
    await published.promise;
    const prompting = service.ingest({ kind: "user-prompt", eventId: "a:prompt", sessionId: "a", session: session("a"), text: "hi" });
    clock.now += 60_000;
    // `a` has been quiet since its start as the sweep looks, so its close waits behind the prompt in the same lane.
    const sweeping = service.closeIdleSessions();
    publishing.resolve();
    await Promise.all([starting, prompting, sweeping]);
    expect(service.openRuntimes()).toBe(1);
    expect(ended("a")).toBeNull();
  });

  it("looks for quiet sessions every minute on its own", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      const { clock, service, start, ended } = await idleService();
      await start("a");
      clock.now += 60_000;
      vi.advanceTimersByTime(60_000);
      await vi.waitFor(() => expect(service.openRuntimes()).toBe(0));
      expect(ended("a")).not.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("leaves no timer behind once it has closed, or when it could not listen", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      const root = await mkdtemp(join(tmpdir(), "rerail-idle-timer-"));
      roots.push(root);
      const socketPath = process.platform === "win32" ? `\\\\.\\pipe\\rerail-idle-timer-${process.pid}-${Date.now()}` : join(root, "engine.sock");
      const options = { socketPath, storageRoot: join(root, "state"), databasePath: join(root, "rerail.sqlite"), embedder: testEmbedder };
      const service = await startEngineService(options);
      expect(vi.getTimerCount()).toBe(1);
      // A second service cannot listen on the socket the first holds.
      await expect(startEngineService(options)).rejects.toThrow();
      expect(vi.getTimerCount()).toBe(1);
      await service.close();
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
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
