import { afterEach, describe, expect, it, vi } from "vitest";
const nativeBoundary = vi.hoisted(() => ({
  startDaemonForPackage: vi.fn(),
  isMainRerailBuild: vi.fn<() => boolean>(),
}));
vi.mock("../../src/runtime/daemon-control.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../../src/runtime/daemon-control.js"),
  startDaemonForPackage: nativeBoundary.startDaemonForPackage,
}));
vi.mock("../../src/engine/native/rerail-build.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../../src/engine/native/rerail-build.js"),
  isMainRerailBuild: nativeBoundary.isMainRerailBuild,
}));
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import superjson from "superjson";
import type { AdapterDelivery } from "../../src/engine/index.js";
import type { DaemonLaunchOptions } from "../../src/runtime/index.js";
import { createNativeSession, NativeSessionClient } from "../../src/harness/native-session-client.js";
import { MUTATION_ENTRY_TYPE } from "../../src/runtime/index.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "../support/native-daemon.js";

const delivery = (content: string, ruleIds: readonly string[] = []): AdapterDelivery => ({
  sessionId: "session", requested: "steer", mechanism: "native-queue", content, ruleIds,
});

interface NativeRequestBody {
  operation: string;
  entries?: unknown;
  entriesEncoding?: string;
  input?: unknown;
}

const directories: string[] = [];

const readBody = (init: RequestInit): NativeRequestBody => JSON.parse(String(init.body)) as NativeRequestBody;

afterEach(async () => {
  await closeNativeTestDaemons();
  nativeBoundary.startDaemonForPackage.mockReset();
  nativeBoundary.isMainRerailBuild.mockReset();
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  vi.useRealTimers();
  for (const path of directories.splice(0)) await rm(path, { recursive: true, force: true });
});

describe("daemon native client delivery lifecycle", () => {
  it("buffers startup until activation, orders background delivery behind events, and drains shutdown entries", async () => {
    vi.useFakeTimers();
    const entered = Promise.withResolvers<void>();
    const release = Promise.withResolvers<void>();
    const received: string[] = [];
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = JSON.parse(String(init.body)) as { operation: string };
      if (request.operation === "ingest") {
        entered.resolve();
        await release.promise;
      }
      return Response.json({
        deliveries: request.operation === "initialize" ? [delivery("startup")]
          : request.operation === "ingest" ? [delivery("event")]
          : request.operation === "poll" ? [delivery("background")] : [],
        entries: request.operation === "close" ? [{ customType: "closed", data: 1 }] : [],
      });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "session", entries: () => [],
      appendEntry: type => { received.push(type); },
      deliver: (_client, deliveries) => { received.push(...deliveries.map(item => item.content)); },
    });
    await client.initialize({});
    expect(received).toEqual([]);
    await client.activate();
    const ingesting = client.ingest({ kind: "user-prompt", sessionId: "session", eventId: "event", text: "hello" });
    await entered.promise;
    await vi.advanceTimersByTimeAsync(100);
    expect(received).toEqual(["startup"]);
    const closing = client.close();
    release.resolve();
    await Promise.all([ingesting, closing]);
    expect(received).toEqual(["startup", "event", "background", "closed"]);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(received).toEqual(["startup", "event", "background", "closed"]);
  });

  it("starts and leaves the shared fallback daemon in bundled UI mode", async () => {
    let sharedUi: "bundled" | "dev" | undefined;
    const active: NativeSessionClient[] = [];
    nativeBoundary.startDaemonForPackage.mockImplementation(async (_control: unknown, options: DaemonLaunchOptions) => {
      sharedUi = options.dev ? "dev" : "bundled";
      return {
        started: true,
        daemon: {
          state: { url: "http://127.0.0.1:4737", nativeToken: "private-token", ui: sharedUi },
          health: { workspaceRoot: "/workspace" },
        },
      };
    });
    vi.stubGlobal("fetch", async () => Response.json({ deliveries: [], entries: [] }));

    await expect(createNativeSession({
      current: () => undefined,
      matchesCurrent: () => false,
      close: async () => undefined,
      activate(client) { active.push(client); },
      entries: () => [],
      appendEntry() {},
      deliver() {},
      reattach: async () => undefined,
    }, {}, {
      host: "omp",
      sessionId: "fallback-ui",
      cwd: "/workspace",
    }, false, {})).resolves.toBe("attached");

    expect(sharedUi).toBe("bundled");
    await active[0]?.close();
    expect(sharedUi).toBe("bundled");
  });
});

describe("pi session routing", () => {
  const piSession = { host: "pi" as const, sessionId: "pi-session", cwd: "/workspace" };
  const piHost = (active: NativeSessionClient[]) => ({
    current: () => undefined,
    matchesCurrent: () => false,
    close: async () => undefined,
    activate(client: NativeSessionClient) { active.push(client); },
    entries: () => [],
    appendEntry() {},
    deliver() {},
    reattach: async () => undefined,
  });

  it("sends a main build's pi session to the daemon, since an in-process main runtime would be a second main instance", async () => {
    const active: NativeSessionClient[] = [];
    const initialized: Array<{ host?: unknown; operation?: unknown }> = [];
    nativeBoundary.isMainRerailBuild.mockReturnValue(true);
    nativeBoundary.startDaemonForPackage.mockResolvedValue({
      started: false,
      daemon: { state: { url: "http://127.0.0.1:4737", nativeToken: "private-token" }, health: { workspaceRoot: "/workspace" } },
    });
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      initialized.push(JSON.parse(String(init.body)) as { host?: unknown; operation?: unknown });
      return Response.json({ deliveries: [], entries: [] });
    });

    await expect(createNativeSession(piHost(active), {}, piSession, false, {})).resolves.toBe("attached");

    expect(nativeBoundary.startDaemonForPackage).toHaveBeenCalledOnce();
    expect(initialized[0]).toEqual(expect.objectContaining({ host: "pi", operation: "initialize" }));
    await active[0]?.close();
  });

  it("keeps a checkout's pi session in-process and starts no daemon", async () => {
    const active: NativeSessionClient[] = [];
    nativeBoundary.isMainRerailBuild.mockReturnValue(false);

    await expect(createNativeSession(piHost(active), {}, piSession, false, {})).resolves.toBe("host");

    expect(nativeBoundary.startDaemonForPackage).not.toHaveBeenCalled();
    expect(active).toEqual([]);
  });
});

describe("daemon native client branch encoding", () => {
  it("carries non-JSON branch entries across the native boundary", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-native-branch-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const daemon = await startNativeTestDaemon(root);
    const rule = {
      id: "branch-encoded", mode: "steer", reference: "Keep the branch encoded.",
      match: { operator: "gte", threshold: 0.99 },
      sample: { source: "assistant-text", boundary: "sentence", minimumTokens: 1 },
      action: { content: "Encode the branch." },
      repeat: { mode: "always" },
    };
    const branch = [
      {
        type: "custom", id: "mutation-1", timestamp: new Date(0).toISOString(),
        customType: MUTATION_ENTRY_TYPE, data: { operation: "upsert", rule, replace: false },
      },
      {
        type: "custom", id: "sample-1", timestamp: new Date(0).toISOString(),
        customType: "rerail:non-json",
        data: { count: 7n, observedAt: new Date("2026-01-01T00:00:00.000Z"), lanes: new Map([["left", 1]]) },
      },
    ];
    expect(() => JSON.stringify(branch)).toThrow(TypeError);
    const bodies: NativeRequestBody[] = [];
    const network = globalThis.fetch;
    vi.stubGlobal("fetch", async (url: URL, init: RequestInit) => {
      bodies.push(readBody(init));
      return network(url, init);
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: root, url: new URL(daemon.url) },
      token: daemon.state().nativeToken!,
      host: "omp", sessionId: "branch-encoding", entries: () => branch,
      appendEntry() {}, deliver() {},
    });
    try {
      await client.initialize({ cwd: root, modelPath: resolve("model"), rules: [] });
      const listed = await client.conversationRules({ action: "list" }) as { rules: Array<{ rule: { id: string } }> };
      expect(listed.rules.map(entry => entry.rule.id)).toContain("branch-encoded");
    } finally {
      await client.close();
    }
    const initialize = bodies.find(body => body.operation === "initialize");
    expect(initialize?.entriesEncoding).toBe("superjson");
    const decoded = superjson.deserialize(initialize?.entries as Parameters<typeof superjson.deserialize>[0]) as typeof branch;
    expect(decoded[1]?.data).toEqual({
      count: 7n, observedAt: new Date("2026-01-01T00:00:00.000Z"), lanes: new Map([["left", 1]]),
    });
  });
});

describe("daemon native client polling resilience", () => {
  it("retries a transient poll failure and keeps later requests working", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const received: string[] = [];
    let polls = 0;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      if (request.operation !== "poll") return Response.json({ deliveries: [], entries: [], result: { ok: true } });
      polls += 1;
      if (polls === 1) throw new TypeError("fetch failed");
      return Response.json({ deliveries: [delivery("background")], entries: [] });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "transient", entries: () => [],
      appendEntry() {},
      deliver: (_client, deliveries) => { received.push(...deliveries.map(item => item.content)); },
    });
    await client.initialize({});
    await client.activate();

    await vi.advanceTimersByTimeAsync(100);
    expect(polls).toBe(1);
    expect(received).toEqual([]);
    await vi.advanceTimersByTimeAsync(100);

    expect(received).toEqual(["background"]);
    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("Rerail native runtime unavailable"));
    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("Rerail native runtime recovered"));
    await expect(client.conversationRules({ action: "list" })).resolves.toEqual({ ok: true });
    await client.close();
  });

  it("reinitializes the retained session after the daemon drops an expired lease", async () => {
    vi.useFakeTimers();
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const branch = [{ type: "custom", id: "entry-1", timestamp: new Date(0).toISOString() }];
    const received: string[] = [];
    const bodies: NativeRequestBody[] = [];
    let resident = true;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      bodies.push(request);
      if (request.operation === "initialize") {
        resident = true;
        return Response.json({ deliveries: [], entries: [] });
      }
      if (!resident) {
        return Response.json({ error: "native-request-invalid", message: "native session is not initialized" }, { status: 400 });
      }
      if (request.operation === "poll") return Response.json({ deliveries: [delivery("resumed")], entries: [] });
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "expired", entries: () => branch,
      appendEntry() {},
      deliver: (_client, deliveries) => { received.push(...deliveries.map(item => item.content)); },
    });
    await client.initialize({ cwd: "/workspace" });
    await client.activate();
    resident = false;

    await vi.advanceTimersByTimeAsync(300);

    expect(bodies.map(body => body.operation).slice(0, 4)).toEqual(["initialize", "poll", "initialize", "poll"]);
    expect(received).toEqual(["resumed"]);
    const reinitialization = bodies.filter(body => body.operation === "initialize")[1];
    expect(reinitialization?.input).toEqual({ cwd: "/workspace" });
    expect(superjson.deserialize(reinitialization?.entries as Parameters<typeof superjson.deserialize>[0])).toEqual(branch);
    await expect(client.semantic({ action: "select" })).resolves.toEqual({ ok: true });
    await client.close();
  });

  it("keeps probing when the replayed initialize of a dropped session is itself rejected", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const received: string[] = [];
    const operations: string[] = [];
    let resident = true;
    let rejectReplays = 1;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      operations.push(request.operation);
      if (request.operation === "initialize") {
        if (operations.length > 1 && rejectReplays > 0) {
          rejectReplays -= 1;
          return Response.json({ error: "native-request-invalid", message: "daemon still restoring" }, { status: 500 });
        }
        resident = true;
        return Response.json({ deliveries: [], entries: [] });
      }
      if (!resident) {
        return Response.json({ error: "native-request-invalid", message: "native session is not initialized" }, { status: 400 });
      }
      return Response.json({ deliveries: request.operation === "poll" ? [delivery("resumed")] : [], entries: [] });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "restoring", entries: () => [],
      appendEntry() {},
      deliver: (_client, deliveries) => { received.push(...deliveries.map(item => item.content)); },
    });
    await client.initialize({});
    await client.activate();
    resident = false;

    // poll fails at 100, the replay is rejected at 200.
    await vi.advanceTimersByTimeAsync(250);
    expect(operations).toEqual(["initialize", "poll", "initialize"]);
    // The stream stays shut between the rejected replay and the next probe: the event is dropped
    // rather than sent to a daemon that has no session for it, and it does not count as a probe failure.
    await expect(client.ingest({ kind: "user-prompt", sessionId: "restoring", eventId: "mid", text: "mid" })).resolves.toBeUndefined();
    expect(operations).toEqual(["initialize", "poll", "initialize"]);
    // One failure per probe: the re-armed replay backs off 200 and succeeds at 400, polling resumes at 500.
    await vi.advanceTimersByTimeAsync(250);
    expect(operations).toEqual(["initialize", "poll", "initialize", "initialize", "poll"]);
    expect(received).toEqual(["resumed"]);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("recovered; 1 session event(s)"));
    await client.close();
  });

  it("reports an outage once, drops session events while shut, and resumes after the daemon heals", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const operations: string[] = [];
    let healthy = true;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      operations.push(request.operation);
      if (!healthy && request.operation !== "initialize") {
        return Response.json({ error: "native-request-invalid", message: "database disk image is malformed" }, { status: 400 });
      }
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "outage", entries: () => [],
      appendEntry() {},
      deliver() {},
    });
    await client.initialize({});
    await client.activate();
    healthy = false;
    const event = (eventId: string) => ({ kind: "user-prompt" as const, sessionId: "outage", eventId, text: eventId });

    // The first failure reports; every later event resolves quietly instead of surfacing to the host.
    await expect(client.ingest(event("first"))).resolves.toBeUndefined();
    await expect(client.ingest(event("second"))).resolves.toBeUndefined();
    await expect(client.ingest(event("third"))).resolves.toBeUndefined();
    expect(operations.filter(operation => operation === "ingest")).toEqual(["ingest"]);
    expect(stderr).toHaveBeenCalledTimes(1);
    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("database disk image is malformed"));

    // Probes back off while the daemon stays broken, and explicit tools still see the real fault.
    await vi.advanceTimersByTimeAsync(100 + 200 + 400);
    expect(operations.filter(operation => operation === "poll")).toHaveLength(3);
    await expect(client.conversationRules({ action: "list" })).rejects.toThrow("database disk image is malformed");
    expect(stderr).toHaveBeenCalledTimes(1);

    healthy = true;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(stderr).toHaveBeenCalledTimes(2);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("3 session event(s) were dropped"));
    await expect(client.ingest(event("fourth"))).resolves.toEqual({ ok: true });
    expect(operations.filter(operation => operation === "ingest")).toEqual(["ingest", "ingest"]);
    // Recovery resumes exactly one poll loop.
    const polls = operations.filter(operation => operation === "poll").length;
    await vi.advanceTimersByTimeAsync(1_000);
    expect(operations.filter(operation => operation === "poll")).toHaveLength(polls + 10);
    await client.close();
  });

  it("lets an explicit operation fail on a healthy daemon without shutting the stream", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const operations: string[] = [];
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      operations.push(request.operation);
      if (request.operation === "cosine") return Response.json({ error: "native-request-invalid", message: "unknown action" }, { status: 400 });
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "explicit", entries: () => [],
      appendEntry() {},
      deliver() {},
    });
    await client.initialize({});
    await client.activate();
    await expect(client.conversationRules({ action: "bogus" })).rejects.toThrow("unknown action");
    await expect(client.ingest({ kind: "user-prompt", sessionId: "explicit", eventId: "next", text: "next" })).resolves.toEqual({ ok: true });
    await vi.advanceTimersByTimeAsync(300);
    expect(operations.filter(operation => operation === "poll")).toHaveLength(3);
    expect(stderr).not.toHaveBeenCalled();
    await client.close();
  });

  it("rejects a daemon reply that names a requested kind outside the rule modes instead of delivering it", async () => {
    const delivered: AdapterDelivery[] = [];
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const retired = readBody(init).operation === "cosine" ? [{ ...delivery("Recorded only."), requested: "observe" }] : [];
      return Response.json({ deliveries: retired, entries: [], result: null });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "skewed", entries: () => [],
      appendEntry() {},
      deliver(_client, deliveries) { delivered.push(...deliveries); },
    });
    await client.initialize({});
    await client.activate();
    await expect(client.conversationRules({ action: "list" })).rejects.toThrow("Rerail native response has an invalid shape");
    expect(delivered).toEqual([]);
    await client.close();
  });

  it("hands a persistent outage and a rejected token to the host for a rebuild", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const operations: string[] = [];
    let mode: "down" | "new-token" | "healthy" = "healthy";
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      operations.push(readBody(init).operation);
      if (mode === "down") throw new TypeError("fetch failed");
      if (mode === "new-token") return Response.json({ error: "native-authorization-invalid" }, { status: 401 });
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const reattach = vi.fn(async () => {
      if (mode === "down") throw new Error("daemon still starting");
      // The host resolved a live daemon: with a rejected token it retires this client for a new one.
      if (mode === "new-token") await client.close();
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "rebuild", entries: () => [],
      appendEntry() {},
      deliver() {},
      reattach,
    });
    await client.initialize({});
    await client.activate();
    const event = (eventId: string) => ({ kind: "user-prompt" as const, sessionId: "rebuild", eventId, text: eventId });

    mode = "down";
    await expect(client.ingest(event("one"))).resolves.toBeUndefined();
    // Two cheap poll probes first, then the host is asked to rebuild while the daemon is still down.
    await vi.advanceTimersByTimeAsync(100 + 200);
    expect(operations.filter(operation => operation === "poll")).toHaveLength(2);
    expect(reattach).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(400);
    expect(reattach).toHaveBeenCalledTimes(1);
    await expect(client.ingest(event("two"))).resolves.toBeUndefined();
    expect(operations.filter(operation => operation === "ingest")).toEqual(["ingest"]);

    // A retried rebuild waits far longer than a poll, and the one that succeeds ends the outage
    // even though this instance never spoke to the daemon again.
    mode = "healthy";
    await vi.advanceTimersByTimeAsync(5_000);
    expect(reattach).toHaveBeenCalledTimes(2);
    expect(stderr).toHaveBeenCalledTimes(2);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("recovered; 2 session event(s)"));
    await expect(client.ingest(event("three"))).resolves.toEqual({ ok: true });

    // A restarted daemon rejects the old token: no polling, straight to the host, which retires the client.
    mode = "new-token";
    await expect(client.ingest(event("four"))).resolves.toBeUndefined();
    expect(stderr).toHaveBeenCalledTimes(3);
    await vi.advanceTimersByTimeAsync(100);
    expect(reattach).toHaveBeenCalledTimes(3);
    expect(operations.filter(operation => operation === "poll")).toHaveLength(2);
    expect(operations.at(-1)).toBe("close");
    expect(stderr).toHaveBeenCalledTimes(4);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("recovered; 1 session event(s)"));
  });

  it("stops rebuilding once the budget is spent and names the way back", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const operations: string[] = [];
    let healthy = true;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      operations.push(readBody(init).operation);
      if (!healthy) throw new TypeError("fetch failed");
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const reattach = vi.fn(async () => { throw new Error("daemon still starting"); });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "gone", entries: () => [],
      appendEntry() {},
      deliver() {},
      reattach,
    });
    await client.initialize({});
    await client.activate();
    healthy = false;
    const event = (eventId: string) => ({ kind: "user-prompt" as const, sessionId: "gone", eventId, text: eventId });

    await expect(client.ingest(event("one"))).resolves.toBeUndefined();
    expect(stderr).toHaveBeenCalledTimes(1);
    // Two cheap polls, then the first rebuild on the same cadence.
    await vi.advanceTimersByTimeAsync(100 + 200 + 400);
    expect(reattach).toHaveBeenCalledTimes(1);

    // Well past the 5s and 10s the rest of the budget is spaced over: three rebuilds, then it stops.
    await vi.advanceTimersByTimeAsync(30_000);
    expect(reattach).toHaveBeenCalledTimes(3);
    expect(stderr).toHaveBeenCalledTimes(2);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("could not restart its daemon after 3 attempts"));
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("daemon still starting"));
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("/rerail-start"));

    // Terminal: minutes pass without another spawn or another word, and the stream stays shut, so
    // events are dropped and counted rather than sent to a daemon that is not there.
    const requests = operations.length;
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(reattach).toHaveBeenCalledTimes(3);
    expect(stderr).toHaveBeenCalledTimes(2);
    await expect(client.ingest(event("two"))).resolves.toBeUndefined();
    expect(operations).toHaveLength(requests);
    await client.close();
  });

  it("recovers on the last rebuild the budget allows and gives the next outage a fresh one", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    let healthy = true;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      readBody(init);
      if (!healthy) throw new TypeError("fetch failed");
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    let attempts = 0;
    // Only the third rebuild — the last one the budget allows — finds a daemon.
    const reattach = vi.fn(async () => {
      attempts += 1;
      if (attempts !== 3) throw new Error("daemon still starting");
      healthy = true;
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "late", entries: () => [],
      appendEntry() {},
      deliver() {},
      reattach,
    });
    await client.initialize({});
    await client.activate();
    healthy = false;
    const event = (eventId: string) => ({ kind: "user-prompt" as const, sessionId: "late", eventId, text: eventId });

    await expect(client.ingest(event("one"))).resolves.toBeUndefined();
    await vi.advanceTimersByTimeAsync(30_000);
    expect(reattach).toHaveBeenCalledTimes(3);
    expect(stderr).toHaveBeenCalledTimes(2);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("recovered; 1 session event(s)"));
    await expect(client.ingest(event("two"))).resolves.toEqual({ ok: true });

    // The healed client starts over: the next outage buys three rebuilds of its own before it stops.
    healthy = false;
    await expect(client.ingest(event("three"))).resolves.toBeUndefined();
    expect(stderr).toHaveBeenCalledTimes(3);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(reattach).toHaveBeenCalledTimes(6);
    expect(stderr).toHaveBeenCalledTimes(4);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("could not restart its daemon after 3 attempts"));
    await vi.advanceTimersByTimeAsync(10 * 60_000);
    expect(reattach).toHaveBeenCalledTimes(6);
    await client.close();
  });
});
