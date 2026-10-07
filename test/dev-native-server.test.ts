import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DevNativeServer } from "../src/dev-native-server.js";
import { SqliteTranscriptReader } from "../src/transcript-store.js";

describe("DevNativeServer request boundary", () => {
  let root: string | undefined;
  let server: DevNativeServer | undefined;

  afterEach(async () => {
    await server?.close();
    vi.useRealTimers();
    if (root !== undefined) await rm(root, { recursive: true, force: true });
    vi.unstubAllEnvs();
  });

  it("rejects a request addressed to another checkout", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    server = new DevNativeServer({
      workspaceRoot: root,
      modelPath: join(root, "model"),
      databasePath: join(root, "rerail.sqlite"),
    });

    await expect(server.handle({
      workspaceRoot: join(root, "other"), sessionId: "s", host: "omp", operation: "close",
    })).rejects.toThrow("native workspace mismatch");
    await server.close();
  });

  it("counts acknowledged host handoffs once in the session HUD", async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const databasePath = join(root, "rerail.sqlite");
    server = new DevNativeServer({
      workspaceRoot: root,
      modelPath: resolve("model"),
      databasePath,
    });
    const reader = new SqliteTranscriptReader(databasePath);
    const session = { workspaceRoot: root, sessionId: "s", host: "omp" as const };
    try {
      await server.handle({ ...session, operation: "initialize", entries: [] });
      expect(reader.hud("s").steeringCount).toBe(0);
      // A client without re-rail attribution sends the bare receipt id.
      const receipt = { ...session, operation: "record-steering-delivery" as const, input: "host-handoff" };
      await server.handle(receipt);
      await server.handle(receipt);
      expect(reader.hud("s").steeringCount).toBe(1);
      await server.handle({
        ...session,
        operation: "record-steering-delivery" as const,
        input: { receiptId: "attributed-handoff", ruleIds: ["rerail:no-raw-sql"] },
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

  it("retires an abandoned session and closes its transcript resources after the lease", async () => {
    vi.useFakeTimers();
    root = await mkdtemp(join(tmpdir(), "rerail-native-server-"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const databasePath = join(root, "rerail.sqlite");
    server = new DevNativeServer({
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
    const reader = new SqliteTranscriptReader(databasePath);
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
    const databasePath = join(root, "rerail.sqlite");
    server = new DevNativeServer({
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

    const reader = new SqliteTranscriptReader(databasePath);
    expect(reader.sessions()).toContainEqual(expect.objectContaining({
      sessionId: "active",
      endedAt: null,
    }));
    reader.close();
  });
});
