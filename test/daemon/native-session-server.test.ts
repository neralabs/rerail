import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NativeSessionServer } from "../../src/daemon/native-session-server.js";
import { SqliteSessionReader } from "../../src/runtime/index.js";
import { CodeRuleBridge } from "../../src/engine/index.js";
import { startTestRepository, stopTestRepository } from "../support/started-repository.js";

describe("NativeSessionServer request boundary", () => {
  let root: string | undefined;
  let server: NativeSessionServer | undefined;

  afterEach(async () => {
    await server?.close();
    vi.useRealTimers();
    if (root !== undefined) await rm(root, { recursive: true, force: true });
    vi.unstubAllEnvs();
  });

  it("rejects a request addressed to another checkout", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    server = new NativeSessionServer({
      workspaceRoot: root,
      modelPath: join(root, "model"),
      databasePath: join(root, "rerail.sqlite"),
    });

    await expect(server.handle({
      workspaceRoot: join(root, "other"), sessionId: "s", host: "omp", operation: "close",
    })).rejects.toThrow("native workspace mismatch");
    await server.close();
  });

  it("counts acknowledged steers once in the session HUD", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    startTestRepository(root, join(root, "rerail.sqlite"));
    const databasePath = join(root, "rerail.sqlite");
    server = new NativeSessionServer({
      workspaceRoot: root,
      modelPath: resolve("model"),
      databasePath,
    });
    const reader = new SqliteSessionReader(databasePath);
    const session = { workspaceRoot: root, sessionId: "s", host: "omp" as const };
    try {
      await server.handle({ ...session, operation: "initialize", entries: [] });
      expect(reader.hud("s").steeringCount).toBe(0);
      const receipt = { ...session, operation: "record-steering-delivery" as const, input: { receiptId: "host-steer", ruleIds: [] } };
      await server.handle(receipt);
      await server.handle(receipt);
      expect(reader.hud("s").steeringCount).toBe(1);
      await server.handle({
        ...session,
        operation: "record-steering-delivery" as const,
        input: { receiptId: "attributed-steer", ruleIds: ["rerail:no-raw-sql"] },
      });
      expect(reader.hud("s").steeringCount).toBe(2);
      expect(reader.events("s").filter(event => event.kind === "steering-delivered").map(event => event.payload)).toEqual([
        { ruleIds: [] },
        { ruleIds: ["rerail:no-raw-sql"] },
      ]);
    } finally {
      reader.close();
    }
  });

  it("writes how long initialize took to answer and how long the code-rule evaluator took to open, with the session id, to the daemon log", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const databasePath = join(root, "rerail.sqlite");
    startTestRepository(root, databasePath);
    server = new NativeSessionServer({ workspaceRoot: root, modelPath: resolve("model"), databasePath });
    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    try {
      await server.handle({ workspaceRoot: root, sessionId: "timed-session", host: "omp", operation: "initialize", entries: [] });
      const lines = () => stderr.mock.calls.map(([chunk]) => String(chunk));
      expect(lines().filter(line => line.includes("initialized"))).toEqual([
        expect.stringMatching(/^rerail: session timed-session initialized in \d+ ms\n$/u),
      ]);
      // The evaluator opens behind the answer.
      await vi.waitFor(() => expect(lines().filter(line => line.includes("code-rule evaluator"))).toEqual([
        expect.stringMatching(/^rerail: code-rule evaluator of session timed-session opened in \d+ ms\n$/u),
      ]));
    } finally {
      stderr.mockRestore();
    }
  });

  it("records a dropped-events gap in the session's history", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const databasePath = join(root, "rerail.sqlite");
    startTestRepository(root, databasePath);
    server = new NativeSessionServer({ workspaceRoot: root, modelPath: resolve("model"), databasePath });
    const reader = new SqliteSessionReader(databasePath);
    const session = { workspaceRoot: root, sessionId: "s", host: "omp" as const };
    const gap = {
      id: "gap-1", count: 3, kinds: { "user-prompt": 1, call: 2 }, firstEventId: "input:1", lastEventId: "tool-call:2",
      firstDroppedAt: 1_000, lastDroppedAt: 2_000,
    };
    try {
      await server.handle({ ...session, operation: "initialize", entries: [] });
      await expect(server.handle({ ...session, operation: "record-gap", input: { ...gap, count: 0 } })).rejects.toThrow();
      await server.handle({ ...session, operation: "record-gap", input: gap });
      // A record sent again after its acknowledgement was lost is stored once.
      await server.handle({ ...session, operation: "record-gap", input: gap });
      expect(reader.events("s").filter(event => event.kind === "events-dropped")).toEqual([
        expect.objectContaining({ eventId: "events-dropped:gap-1", payload: gap }),
      ]);
    } finally {
      reader.close();
    }
  });

  it("answers initialize before the session's code-rule evaluator opens, and holds a tool call until it has", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const databasePath = join(root, "rerail.sqlite");
    startTestRepository(root, databasePath);
    const opening = Promise.withResolvers<void>();
    const open = CodeRuleBridge.prototype.openWorkspace;
    const spy = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace").mockImplementationOnce(async function (this: CodeRuleBridge, sessionId: string) {
      await opening.promise;
      await open.call(this, sessionId);
    });
    server = new NativeSessionServer({ workspaceRoot: root, modelPath: resolve("model"), databasePath });
    const session = { workspaceRoot: root, sessionId: "s", host: "omp" as const };
    try {
      await server.handle({ ...session, operation: "initialize", entries: [] });
      let called = false;
      const call = server.handle({
        ...session, operation: "ingest",
        event: { kind: "call", eventId: "call", sessionId: "s", toolCallId: "write", toolName: "write", input: { path: "a.py" } },
      }).finally(() => { called = true; });
      await new Promise<void>(resolve => setImmediate(resolve));
      expect(called).toBe(false);
      opening.resolve();
      await call;
    } finally {
      opening.resolve();
      spy.mockRestore();
    }
  });

  describe("while the session's code-rule evaluator is still opening", () => {
    async function held() {
      root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
      vi.stubEnv("RERAIL_HOME", join(root, "home"));
      const databasePath = join(root, "rerail.sqlite");
      startTestRepository(root, databasePath);
      const opening = Promise.withResolvers<void>();
      // The held evaluator opens when released; what it opens is not under test, that the bridge it opens for is closed is.
      const openSpy = vi.spyOn(CodeRuleBridge.prototype, "openWorkspace").mockImplementationOnce(async () => { await opening.promise; });
      const closeSpy = vi.spyOn(CodeRuleBridge.prototype, "close");
      server = new NativeSessionServer({ workspaceRoot: root, modelPath: resolve("model"), databasePath });
      return {
        opening, closeSpy, databasePath,
        session: { workspaceRoot: root, sessionId: "s", host: "omp" as const },
        restore: () => { opening.resolve(); openSpy.mockRestore(); closeSpy.mockRestore(); },
      };
    }

    it("closes the session without waiting for it, and the evaluator is closed once it has opened", async () => {
      const { opening, closeSpy, session, restore } = await held();
      try {
        await server!.handle({ ...session, operation: "initialize", entries: [] });
        let closed = false;
        const closing = server!.handle({ ...session, operation: "close" }).then(() => { closed = true; });
        await vi.waitFor(() => expect(closed).toBe(true), { timeout: 20_000 });
        await closing;
        expect(closeSpy).not.toHaveBeenCalled();
        opening.resolve();
        await vi.waitFor(() => expect(closeSpy).toHaveBeenCalledTimes(1), { timeout: 20_000 });
      } finally {
        restore();
      }
    }, 30_000);

    it("replaces the session without waiting for it, and the old evaluator is closed once it has opened", async () => {
      const { opening, closeSpy, databasePath, session, restore } = await held();
      try {
        await server!.handle({ ...session, operation: "initialize", entries: [] });
        let replaced = false;
        const replacing = server!.handle({ ...session, operation: "initialize", entries: [], input: { rebuild: true } }).then(() => { replaced = true; });
        await vi.waitFor(() => expect(replaced).toBe(true), { timeout: 20_000 });
        await replacing;
        expect(closeSpy).not.toHaveBeenCalled();
        opening.resolve();
        await vi.waitFor(() => expect(closeSpy).toHaveBeenCalledTimes(1), { timeout: 20_000 });
        const reader = new SqliteSessionReader(databasePath);
        try {
          expect(reader.sessions().find(recorded => recorded.sessionId === "s")?.endedAt).toBeNull();
        } finally {
          reader.close();
        }
      } finally {
        restore();
      }
    }, 30_000);
  });

  it("closes the session a rebuild replaces while the checkout is stopped", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const databasePath = join(root, "rerail.sqlite");
    startTestRepository(root, databasePath);
    server = new NativeSessionServer({ workspaceRoot: root, modelPath: resolve("model"), databasePath });
    const session = { workspaceRoot: root, sessionId: "s", host: "omp" as const };
    await server.handle({ ...session, operation: "initialize", entries: [] });
    // `/rerail-stop` stops the checkout, then the host reattaches with a rebuild.
    stopTestRepository(root, databasePath);
    await server.handle({ ...session, operation: "initialize", entries: [], input: { rebuild: true } });
    const reader = new SqliteSessionReader(databasePath);
    try {
      expect(reader.sessions().find(recorded => recorded.sessionId === "s")?.endedAt).not.toBeNull();
    } finally {
      reader.close();
    }
  });

  it("retires an abandoned session and closes its session store resources after the lease", async () => {
    vi.useFakeTimers();
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    startTestRepository(root, join(root, "rerail.sqlite"));
    const databasePath = join(root, "rerail.sqlite");
    server = new NativeSessionServer({
      workspaceRoot: root,
      modelPath: resolve("model"),
      databasePath,
      sessionLeaseMs: 1_000,
      sessionSweepIntervalMs: 100,
    });
    const request = { workspaceRoot: root, sessionId: "abandoned", host: "omp" as const };
    await server.handle({
      ...request,
      operation: "initialize",
      entries: [],
      input: { cwd: root, rules: [] },
    });

    await vi.advanceTimersByTimeAsync(1_100);

    await expect(server.handle({ ...request, operation: "poll" })).rejects.toThrow("native session is not initialized");
    const reader = new SqliteSessionReader(databasePath);
    expect(reader.sessions()).toContainEqual(expect.objectContaining({
      sessionId: "abandoned",
      endedAt: expect.any(Number),
    }));
    reader.close();
  });

  it("keeps a resident session alive while its host continues polling", async () => {
    vi.useFakeTimers();
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    startTestRepository(root, join(root, "rerail.sqlite"));
    const databasePath = join(root, "rerail.sqlite");
    server = new NativeSessionServer({
      workspaceRoot: root,
      modelPath: resolve("model"),
      databasePath,
      sessionLeaseMs: 1_000,
      sessionSweepIntervalMs: 100,
    });
    const request = { workspaceRoot: root, sessionId: "active", host: "omp" as const };
    await server.handle({
      ...request,
      operation: "initialize",
      entries: [],
      input: { cwd: root, rules: [] },
    });

    for (let heartbeat = 0; heartbeat < 3; heartbeat += 1) {
      await vi.advanceTimersByTimeAsync(750);
      await expect(server.handle({ ...request, operation: "poll" })).resolves.toMatchObject({
        deliveries: [],
      });
    }

    const reader = new SqliteSessionReader(databasePath);
    expect(reader.sessions()).toContainEqual(expect.objectContaining({
      sessionId: "active",
      endedAt: null,
    }));
    reader.close();
  });
});
