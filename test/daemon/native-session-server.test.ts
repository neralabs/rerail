import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NativeSessionServer } from "../../src/daemon/native-session-server.js";
import { SqliteSessionReader } from "../../src/runtime/index.js";
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
      // A client without steer attribution sends the bare receipt id.
      const receipt = { ...session, operation: "record-steering-delivery" as const, input: "host-steer" };
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
