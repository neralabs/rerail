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
import { mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import superjson from "superjson";
import { z } from "zod";
import type { AdapterDelivery } from "../../src/engine/index.js";
import type { DaemonLaunchOptions } from "../../src/runtime/index.js";
import { createNativeSession, NativeSessionClient } from "../../src/harness/native-session-client.js";
import { CodeRuleBridge, codeRuleSchema, defaultRerailDatabasePath } from "../../src/engine/index.js";
import { EVENTS_DROPPED_ENTRY_TYPE, MUTATION_ENTRY_TYPE, SqliteSessionReader } from "../../src/runtime/index.js";
import { closeNativeTestDaemons, startNativeTestDaemon } from "../support/native-daemon.js";
import { startTestRepository } from "../support/started-repository.js";
import { setUpTestRepository } from "../support/repository-rules.js";

const delivery = (content: string, ruleIds: readonly string[] = []): AdapterDelivery => ({
  sessionId: "session", requested: "steer", mechanism: "native-queue", content, ruleIds, rules: ruleIds.map(id => ({ id, state: "steered" })),
});

interface NativeRequestBody {
  operation: string;
  entries?: unknown;
  entriesEncoding?: string;
  input?: unknown;
}

const directories: string[] = [];

const readBody = (init: RequestInit): NativeRequestBody => JSON.parse(String(init.body)) as NativeRequestBody;
/** The part of a `record-gap` request a test compares across requests and host-session markers. */
const recordedGap = z.object({ id: z.string() }).passthrough();

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
      instance: { workspaceRoot: root, url: new URL(daemon.url), accessToken: daemon.state().accessToken! },
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
  it("waits for a daemon still working on an event, then sends the events queued behind it, dropping none", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const requests: Array<{ operation: string; ticket?: string; eventId?: string }> = [];
    const received: string[] = [];
    let stillWorking = 2;
    const sent = z.object({ operation: z.string(), input: z.object({ ticket: z.string().optional() }).passthrough().optional(), event: z.unknown().optional() });
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = sent.parse(JSON.parse(String(init.body)));
      const eventId = request.event === undefined
        ? undefined
        : z.object({ eventId: z.string() }).parse(superjson.deserialize(request.event as Parameters<typeof superjson.deserialize>[0])).eventId;
      const ticket = request.input?.ticket;
      requests.push({ operation: request.operation, ...(ticket === undefined ? {} : { ticket }), ...(eventId === undefined ? {} : { eventId }) });
      if (eventId === "slow") return Response.json({ pendingReply: "slow-ticket", deliveries: [], entries: [] });
      if (request.operation === "await-reply") {
        stillWorking -= 1;
        return Response.json(stillWorking >= 0
          ? { pendingReply: "slow-ticket", deliveries: [], entries: [] }
          : { deliveries: [delivery("late steer")], entries: [] });
      }
      return Response.json({ deliveries: [], entries: [] });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "busy", entries: () => [],
      appendEntry() {},
      deliver: (_client, deliveries) => { received.push(...deliveries.map(item => item.content)); },
    });
    await client.initialize({});
    await client.activate();
    const prompt = (eventId: string) => ({ kind: "user-prompt" as const, sessionId: "busy", eventId, text: eventId });

    await expect(Promise.all([client.ingest(prompt("slow")), client.ingest(prompt("next"))])).resolves.toEqual([true, true]);
    expect(requests.filter(request => request.operation !== "poll")).toEqual([
      { operation: "initialize" },
      { operation: "ingest", eventId: "slow" },
      { operation: "await-reply", ticket: "slow-ticket" },
      { operation: "await-reply", ticket: "slow-ticket" },
      { operation: "await-reply", ticket: "slow-ticket" },
      { operation: "ingest", eventId: "next" },
    ]);
    expect(received).toEqual(["late steer"]);
    expect(stderr).not.toHaveBeenCalled();
    await client.close();
  });

  it("gives up on an operation the daemon is still working on after two minutes, as an outage that drops what queued behind it", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const operations: string[] = [];
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const { operation } = readBody(init);
      operations.push(operation);
      if (operation === "ingest") return Response.json({ pendingReply: "stuck", deliveries: [], entries: [] });
      // A daemon holds each ask for its reply bound before saying it is still working.
      if (operation === "await-reply") {
        const held = Promise.withResolvers<void>();
        setTimeout(held.resolve, 10_000);
        await held.promise;
        return Response.json({ pendingReply: "stuck", deliveries: [], entries: [] });
      }
      return Response.json({ deliveries: [], entries: [] });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "stuck", entries: () => [],
      appendEntry() {},
      deliver() {},
    });
    await client.initialize({});
    await client.activate();
    const prompt = (eventId: string) => ({ kind: "user-prompt" as const, sessionId: "stuck", eventId, text: eventId });

    const stuck = client.ingest(prompt("stuck"));
    const behind = client.ingest(prompt("behind"));
    await vi.advanceTimersByTimeAsync(119_000);
    expect(stderr).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1_000);
    await expect(Promise.all([stuck, behind])).resolves.toEqual([false, false]);
    expect(operations.filter(operation => operation === "await-reply")).toHaveLength(12);
    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("Rerail daemon was still working on ingest after 120s"));
    await client.close();
  });

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
    await expect(client.ingest({ kind: "user-prompt", sessionId: "restoring", eventId: "mid", text: "mid" })).resolves.toBe(false);
    expect(operations).toEqual(["initialize", "poll", "initialize"]);
    // One failure per probe: the re-armed replay backs off 200 and succeeds at 400, which records the
    // dropped event's gap at once; polling resumes at 500.
    await vi.advanceTimersByTimeAsync(250);
    expect(operations).toEqual(["initialize", "poll", "initialize", "initialize", "record-gap", "poll"]);
    expect(received).toEqual(["resumed"]);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("recovered; 1 session event(s)"));
    await client.close();
  });

  it("reports an outage once, drops session events while shut, records their gap, and resumes after the daemon heals", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const operations: string[] = [];
    const gaps: unknown[] = [];
    const entries: Array<{ customType: string; data: unknown }> = [];
    const reported: number[] = [];
    let healthy = true;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      operations.push(request.operation);
      if (request.operation === "record-gap") gaps.push(request.input);
      if (!healthy && request.operation !== "initialize") {
        return Response.json({ error: "native-request-invalid", message: "database disk image is malformed" }, { status: 400 });
      }
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "outage", entries: () => [],
      appendEntry: (customType, data) => { entries.push({ customType, data }); },
      reportDropped: count => { reported.push(count); },
      deliver() {},
    });
    await client.initialize({});
    await client.activate();
    healthy = false;
    const prompt = (eventId: string) => ({ kind: "user-prompt" as const, sessionId: "outage", eventId, text: eventId });

    // The first failure reports; every later event resolves quietly instead of surfacing to the host.
    await expect(client.ingest(prompt("first"))).resolves.toBe(false);
    await expect(client.ingest({ kind: "call", sessionId: "outage", eventId: "second", toolCallId: "t", toolName: "edit", input: {} })).resolves.toBe(false);
    await expect(client.ingest(prompt("third"))).resolves.toBe(false);
    expect(operations.filter(operation => operation === "ingest")).toEqual(["ingest"]);
    expect(stderr).toHaveBeenCalledTimes(1);
    expect(stderr).toHaveBeenCalledWith(expect.stringContaining("database disk image is malformed"));
    // The host session is marked as the gap opens, and the host is told how many events it holds so far.
    expect(entries).toEqual([{
      customType: EVENTS_DROPPED_ENTRY_TYPE,
      data: expect.objectContaining({ count: 1, kinds: { "user-prompt": 1 }, firstEventId: "first", lastEventId: "first", recovered: false }),
    }]);
    expect(reported).toEqual([1, 2, 3]);

    // Probes back off while the daemon stays broken, and explicit tools still see the real fault.
    await vi.advanceTimersByTimeAsync(100 + 200 + 400);
    expect(operations.filter(operation => operation === "poll")).toHaveLength(3);
    await expect(client.conversationRules({ action: "list" })).rejects.toThrow("database disk image is malformed");
    expect(stderr).toHaveBeenCalledTimes(1);
    expect(gaps).toEqual([]);

    healthy = true;
    await vi.advanceTimersByTimeAsync(5_000);
    expect(stderr).toHaveBeenCalledTimes(2);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("3 session event(s) were dropped"));
    // Recovery closes the gap: the host session's marker gets the final count, and the daemon records it once.
    const gap = {
      id: expect.any(String), count: 3, kinds: { "user-prompt": 2, call: 1 }, firstEventId: "first", lastEventId: "third",
      firstDroppedAt: expect.any(Number), lastDroppedAt: expect.any(Number),
    };
    expect(gaps).toEqual([gap]);
    const recorded = recordedGap.parse(gaps[0]);
    expect(entries.map(entry => entry.data)).toEqual([
      expect.objectContaining({ id: recorded.id, recovered: false }),
      { ...recorded, recovered: true },
    ]);
    expect(entries.every(entry => entry.customType === EVENTS_DROPPED_ENTRY_TYPE)).toBe(true);
    expect(reported.at(-1)).toBe(0);
    await expect(client.ingest(prompt("fourth"))).resolves.toBe(true);
    expect(operations.filter(operation => operation === "ingest")).toEqual(["ingest", "ingest"]);
    // Recovery resumes exactly one poll loop.
    const polls = operations.filter(operation => operation === "poll").length;
    await vi.advanceTimersByTimeAsync(1_000);
    expect(operations.filter(operation => operation === "poll")).toHaveLength(polls + 10);
    expect(gaps).toHaveLength(1);
    await client.close();
  });

  it("keeps an unrecorded gap when reporting it fails and retries it no more often than its backoff allows", async () => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const gaps: Array<{ input: unknown; recorded: boolean }> = [];
    let healthy = true;
    let recordGaps = false;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      if (request.operation === "record-gap") {
        gaps.push({ input: request.input, recorded: recordGaps });
        if (!recordGaps) return Response.json({ error: "native-request-invalid", message: "database is locked" }, { status: 500 });
      } else if (!healthy && request.operation !== "initialize") {
        return Response.json({ error: "native-request-invalid", message: "daemon wedged" }, { status: 500 });
      }
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "unrecorded", entries: () => [],
      appendEntry() {},
      deliver() {},
    });
    await client.initialize({});
    await client.activate();

    // One event dropped; the probe at 100 heals the outage, but the daemon refuses the gap's record.
    healthy = false;
    await client.ingest({ kind: "user-prompt", sessionId: "unrecorded", eventId: "lost-prompt", text: "lost" });
    healthy = true;
    await vi.advanceTimersByTimeAsync(100);
    expect(gaps).toEqual([{ input: expect.objectContaining({ count: 1, kinds: { "user-prompt": 1 } }), recorded: false }]);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("could not record 1 dropped session event(s); it retries after a pause"));

    // Polls succeed every 100ms, but the retry waits out its 1s backoff; the refusal is reported once, not per retry.
    const reported = stderr.mock.calls.length;
    await vi.advanceTimersByTimeAsync(800);
    expect(gaps).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(400);
    expect(gaps).toHaveLength(2);
    // The next wait doubles to 2s.
    await vi.advanceTimersByTimeAsync(1_500);
    expect(gaps).toHaveLength(2);
    expect(stderr).toHaveBeenCalledTimes(reported);
    recordGaps = true;
    await vi.advanceTimersByTimeAsync(1_000);
    const firstGapId = recordedGap.parse(gaps[0]!.input).id;
    expect(gaps).toHaveLength(3);
    expect(gaps.at(-1)).toEqual({ input: expect.objectContaining({ id: firstGapId, count: 1, kinds: { "user-prompt": 1 } }), recorded: true });
    const attempts = gaps.length;
    await vi.advanceTimersByTimeAsync(1_000);
    expect(gaps).toHaveLength(attempts);
    await client.close();
    expect(gaps).toHaveLength(attempts);
  });

  it("sends close five seconds after a daemon that never answers the closing gap record, and still closes", async () => {
    vi.useFakeTimers();
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const operations: string[] = [];
    let wedged = false;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const { operation } = readBody(init);
      operations.push(operation);
      if (wedged && operation === "record-gap") return await new Promise<Response>(() => undefined);
      if (wedged && operation !== "initialize" && operation !== "close") {
        return Response.json({ error: "native-request-invalid", message: "daemon wedged" }, { status: 500 });
      }
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "wedged-close", entries: () => [],
      appendEntry() {},
      deliver() {},
    });
    await client.initialize({});
    await client.activate();

    // The daemon wedges and an event is dropped, so a gap is open when the host quits.
    wedged = true;
    await client.ingest({ kind: "user-prompt", sessionId: "wedged-close", eventId: "lost", text: "lost" });
    operations.length = 0;

    let closed = false;
    const closing = client.close().then(() => { closed = true; });
    await vi.advanceTimersByTimeAsync(4_999);
    expect(operations).toContain("record-gap");
    expect(operations).not.toContain("close");
    expect(closed).toBe(false);

    await vi.advanceTimersByTimeAsync(1);
    expect(operations.at(-1)).toBe("close");
    await closing;
    expect(closed).toBe(true);
  });

  it.each([
    { status: 400, message: "Invalid option: expected one of \"initialize\"|\"ingest\"|\"poll\"|\"close\"" },
    { status: 404, message: "not-found" },
    { status: 422, message: "Too small: expected number to be >0" },
  ])("drops a gap the daemon refuses with $status instead of letting it block the next gap", async ({ status, message }) => {
    vi.useFakeTimers();
    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const recorded: Array<{ id: string; accepted: boolean }> = [];
    const entries: Array<{ recovered?: boolean; id?: string }> = [];
    let healthy = true;
    let refuseRecords = true;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      if (request.operation === "record-gap") {
        recorded.push({ id: recordedGap.parse(request.input).id, accepted: !refuseRecords });
        if (refuseRecords) return Response.json({ error: "native-request-invalid", message }, { status });
      } else if (!healthy && request.operation !== "initialize") {
        return Response.json({ error: "native-request-invalid", message: "daemon wedged" }, { status: 500 });
      }
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "refused", entries: () => [],
      appendEntry: (_customType, data) => { entries.push(data as { recovered?: boolean; id?: string }); },
      deliver() {},
    });
    await client.initialize({});
    await client.activate();
    const prompt = (eventId: string) => ({ kind: "user-prompt" as const, sessionId: "refused", eventId, text: eventId });

    // The first gap's record is refused for good when the outage that opened it heals.
    healthy = false;
    await client.ingest(prompt("first"));
    healthy = true;
    await vi.advanceTimersByTimeAsync(100);
    expect(recorded).toHaveLength(1);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("refused the record of 1 dropped session event(s); it is not retried"));
    const refusedId = recorded[0]!.id;

    // A later gap is recorded, and the refused one is not sent again.
    refuseRecords = false;
    healthy = false;
    await client.ingest(prompt("second"));
    healthy = true;
    await vi.advanceTimersByTimeAsync(100);
    await vi.advanceTimersByTimeAsync(5_000);
    expect(recorded.map(gap => gap.accepted)).toEqual([false, true]);
    expect(recorded[1]!.id).not.toBe(refusedId);
    // The refused gap keeps its host-session marker.
    expect(entries.filter(entry => entry.id === refusedId).map(entry => entry.recovered)).toEqual([false, true]);
    await client.close();
    expect(recorded).toHaveLength(2);
  });

  it("keeps a gap the daemon refuses over the session's state, which a later try finds changed", async () => {
    vi.useFakeTimers();
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const recorded: Array<{ id: string; accepted: boolean }> = [];
    let healthy = true;
    let sessionKnown = false;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      if (request.operation === "record-gap") {
        recorded.push({ id: recordedGap.parse(request.input).id, accepted: sessionKnown });
        if (!sessionKnown) return Response.json({ error: "native-request-invalid", message: "native session is not initialized" }, { status: 400 });
      } else if (!healthy && request.operation !== "initialize") {
        return Response.json({ error: "native-request-invalid", message: "daemon wedged" }, { status: 500 });
      }
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    const client = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "native-token",
      host: "omp", sessionId: "restarting", entries: () => [],
      appendEntry() {},
      deliver() {},
    });
    await client.initialize({});
    await client.activate();
    healthy = false;
    await client.ingest({ kind: "user-prompt", sessionId: "restarting", eventId: "lost", text: "lost" });
    healthy = true;
    await vi.advanceTimersByTimeAsync(100);
    expect(recorded).toEqual([expect.objectContaining({ accepted: false })]);
    sessionKnown = true;
    await vi.advanceTimersByTimeAsync(1_200);
    expect(recorded.map(gap => gap.accepted)).toEqual([false, true]);
    expect(recorded[1]!.id).toBe(recorded[0]!.id);
    await client.close();
  });

  it("closes the gap still open at close and records its final count before the daemon closes the session", async () => {
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const root = await mkdtemp(join(tmpdir(), "rerail-native-close-gap-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    startTestRepository(root);
    const daemon = await startNativeTestDaemon(root);
    const operations: string[] = [];
    let wedged = false;
    const network = globalThis.fetch;
    vi.stubGlobal("fetch", async (url: URL, init: RequestInit) => {
      const { operation } = readBody(init);
      operations.push(operation);
      if (wedged && operation === "ingest") return Response.json({ error: "native-request-invalid", message: "daemon wedged" }, { status: 500 });
      return network(url, init);
    });
    const entries: unknown[] = [];
    const reported: number[] = [];
    const client = new NativeSessionClient({
      instance: { workspaceRoot: root, url: new URL(daemon.url), accessToken: daemon.state().accessToken! },
      token: daemon.state().nativeToken!,
      host: "omp", sessionId: "closing", entries: () => [],
      appendEntry: (_customType, data) => { entries.push(data); },
      reportDropped: count => { reported.push(count); },
      deliver() {},
    });
    await client.initialize({ cwd: root, modelPath: resolve("model"), rules: [] });
    await client.activate();
    await client.ingest({ kind: "session-start", eventId: "start", session: { host: "omp", sessionId: "closing", cwd: root } });

    // The daemon wedges and two events are dropped; the host quits before any probe heals the outage.
    wedged = true;
    await client.ingest({ kind: "user-prompt", sessionId: "closing", eventId: "first", text: "first" });
    await client.ingest({ kind: "turn-end", sessionId: "closing", eventId: "second", text: "second" });
    await client.close();

    const gap = { id: expect.any(String), count: 2, kinds: { "user-prompt": 1, "turn-end": 1 }, firstEventId: "first", lastEventId: "second" };
    expect(operations.slice(-2)).toEqual(["record-gap", "close"]);
    const reader = new SqliteSessionReader(defaultRerailDatabasePath());
    try {
      expect(reader.events("closing").filter(event => event.kind === "events-dropped").map(event => event.payload)).toEqual([
        expect.objectContaining(gap),
      ]);
    } finally {
      reader.close();
    }
    // The host session's marker gets the final count too; the footer stops counting with this runtime.
    expect(entries).toEqual([
      expect.objectContaining({ count: 1, recovered: false }),
      expect.objectContaining({ ...gap, recovered: true }),
    ]);
    expect(reported).toEqual([1, 2, 0]);
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
    await expect(client.ingest({ kind: "user-prompt", sessionId: "explicit", eventId: "next", text: "next" })).resolves.toBe(true);
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
    await expect(client.ingest(event("one"))).resolves.toBe(false);
    // Two cheap poll probes first, then the host is asked to rebuild while the daemon is still down.
    await vi.advanceTimersByTimeAsync(100 + 200);
    expect(operations.filter(operation => operation === "poll")).toHaveLength(2);
    expect(reattach).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(400);
    expect(reattach).toHaveBeenCalledTimes(1);
    await expect(client.ingest(event("two"))).resolves.toBe(false);
    expect(operations.filter(operation => operation === "ingest")).toEqual(["ingest"]);

    // A retried rebuild waits far longer than a poll, and the one that succeeds ends the outage
    // even though this instance never spoke to the daemon again.
    mode = "healthy";
    await vi.advanceTimersByTimeAsync(5_000);
    expect(reattach).toHaveBeenCalledTimes(2);
    expect(stderr).toHaveBeenCalledTimes(2);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("recovered; 2 session event(s)"));
    await expect(client.ingest(event("three"))).resolves.toBe(true);

    // A restarted daemon rejects the old token: no polling, straight to the host, which retires the client.
    mode = "new-token";
    await expect(client.ingest(event("four"))).resolves.toBe(false);
    expect(stderr).toHaveBeenCalledTimes(3);
    await vi.advanceTimersByTimeAsync(100);
    expect(reattach).toHaveBeenCalledTimes(3);
    expect(operations.filter(operation => operation === "poll")).toHaveLength(2);
    expect(operations.at(-1)).toBe("close");
    expect(stderr).toHaveBeenCalledTimes(4);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("recovered; 1 session event(s)"));
  });

  it("hands the gap of a runtime retired by a restarted daemon to its replacement in the same session", async () => {
    vi.useFakeTimers();
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const recorded: Array<{ token: string | null; input: unknown }> = [];
    const entries: Array<{ customType: string; data: unknown }> = [];
    let restarted = false;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      const token = new Headers(init.headers).get("authorization");
      // The daemon restarted: it no longer knows the old token, so the retiring runtime cannot record its gap there.
      if (restarted && token === "Bearer old-token") return Response.json({ error: "native-authorization-invalid" }, { status: 401 });
      if (request.operation === "record-gap") recorded.push({ token, input: request.input });
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    nativeBoundary.startDaemonForPackage.mockResolvedValue({
      started: false,
      daemon: { state: { url: "http://127.0.0.1:4737", nativeToken: "new-token" }, health: { workspaceRoot: "/workspace" } },
    });
    const appendEntry = (customType: string, data: unknown): void => { entries.push({ customType, data }); };
    const retired = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "old-token",
      host: "omp", sessionId: "restarted", entries: () => [],
      appendEntry,
      deliver() {},
    });
    await retired.initialize({});
    await retired.activate();
    restarted = true;
    await retired.ingest({ kind: "user-prompt", sessionId: "restarted", eventId: "lost", text: "lost" });
    const active: NativeSessionClient[] = [];

    await expect(createNativeSession({
      current: () => retired,
      matchesCurrent: () => true,
      close: () => retired.close().then(() => undefined),
      activate(client) { active.push(client); },
      entries: () => [],
      appendEntry,
      deliver() {},
      reattach: async () => undefined,
    }, {}, { host: "omp", sessionId: "restarted", cwd: "/workspace" }, false, {})).resolves.toBe("attached");
    await vi.advanceTimersByTimeAsync(0);

    expect(active).toHaveLength(1);
    expect(recorded).toEqual([{ token: "Bearer new-token", input: expect.objectContaining({ count: 1, firstEventId: "lost" }) }]);
    expect(entries.map(entry => entry.data)).toEqual([
      expect.objectContaining({ count: 1, recovered: false }),
      expect.objectContaining({ count: 1, recovered: true }),
    ]);
    await active[0]!.close();
  });

  it("hands over an event the retired runtime drops while it drains its last requests, leaving the footer to its replacement", async () => {
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const recorded: Array<{ token: string | null; input: unknown }> = [];
    const footer: number[] = [];
    const inFlight = Promise.withResolvers<Response>();
    let restarted = false;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      const token = new Headers(init.headers).get("authorization");
      if (restarted && token === "Bearer old-token") {
        // The old daemon answers this ingest only once the replacement has begun retiring its runtime; it never takes the gap.
        if (request.operation === "ingest") return await inFlight.promise;
        return Response.json({ error: "native-authorization-invalid" }, { status: 401 });
      }
      if (request.operation === "record-gap") recorded.push({ token, input: request.input });
      return Response.json({ deliveries: [], entries: [], result: { ok: true } });
    });
    nativeBoundary.startDaemonForPackage.mockResolvedValue({
      started: false,
      daemon: { state: { url: "http://127.0.0.1:4737", nativeToken: "new-token" }, health: { workspaceRoot: "/workspace" } },
    });
    const retired = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "old-token",
      host: "omp", sessionId: "draining", entries: () => [],
      appendEntry() {},
      reportDropped: count => { footer.push(count); },
      deliver() {},
    });
    await retired.initialize({});
    await retired.activate();
    restarted = true;
    const lost = retired.ingest({ kind: "user-prompt", sessionId: "draining", eventId: "in-flight", text: "lost" });
    const active: NativeSessionClient[] = [];

    await expect(createNativeSession({
      current: () => retired,
      matchesCurrent: () => true,
      close: () => {
        const closing = retired.close().then(() => undefined);
        inFlight.resolve(Response.json({ error: "native-authorization-invalid" }, { status: 401 }));
        return closing;
      },
      activate(client) { active.push(client); },
      entries: () => [],
      appendEntry() {},
      deliver() {},
      reattach: async () => undefined,
    }, {}, { host: "omp", sessionId: "draining", cwd: "/workspace" }, false, {})).resolves.toBe("attached");
    await lost;
    // Only the replacement, with the new token, records the gap: the retired runtime's own record was refused.
    await vi.waitFor(() => expect(recorded).toEqual([{ token: "Bearer new-token", input: expect.objectContaining({ count: 1, firstEventId: "in-flight" }) }]));
    expect(footer).toEqual([]);
    await active[0]!.close();
  });

  it("still records the gaps a retired runtime handed over when the replacement's activation fails", async () => {
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const recorded: Array<{ token: string | null; input: unknown }> = [];
    let restarted = false;
    vi.stubGlobal("fetch", async (_url: URL, init: RequestInit) => {
      const request = readBody(init);
      const token = new Headers(init.headers).get("authorization");
      if (restarted && token === "Bearer old-token") return Response.json({ error: "native-authorization-invalid" }, { status: 401 });
      if (request.operation === "record-gap") recorded.push({ token, input: request.input });
      // The replacement's start carries a delivery the host then refuses.
      const deliveries = request.operation === "initialize" && token === "Bearer new-token" ? [delivery("steer")] : [];
      return Response.json({ deliveries, entries: [], result: { ok: true } });
    });
    nativeBoundary.startDaemonForPackage.mockResolvedValue({
      started: false,
      daemon: { state: { url: "http://127.0.0.1:4737", nativeToken: "new-token" }, health: { workspaceRoot: "/workspace" } },
    });
    const retired = new NativeSessionClient({
      instance: { workspaceRoot: "/workspace", url: new URL("http://127.0.0.1:4737") },
      token: "old-token",
      host: "omp", sessionId: "refused", entries: () => [],
      appendEntry() {},
      deliver() {},
    });
    await retired.initialize({});
    await retired.activate();
    restarted = true;
    await retired.ingest({ kind: "user-prompt", sessionId: "refused", eventId: "lost", text: "lost" });

    await expect(createNativeSession({
      current: () => retired,
      matchesCurrent: () => true,
      close: () => retired.close().then(() => undefined),
      activate() {},
      entries: () => [],
      appendEntry() {},
      deliver() { throw new Error("host refused the delivery"); },
      reattach: async () => undefined,
    }, {}, { host: "omp", sessionId: "refused", cwd: "/workspace" }, false, {})).rejects.toThrow("host refused the delivery");

    expect(recorded).toEqual([{ token: "Bearer new-token", input: expect.objectContaining({ count: 1, firstEventId: "lost" }) }]);
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

    await expect(client.ingest(event("one"))).resolves.toBe(false);
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
    await expect(client.ingest(event("two"))).resolves.toBe(false);
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

    await expect(client.ingest(event("one"))).resolves.toBe(false);
    await vi.advanceTimersByTimeAsync(30_000);
    expect(reattach).toHaveBeenCalledTimes(3);
    expect(stderr).toHaveBeenCalledTimes(2);
    expect(stderr).toHaveBeenLastCalledWith(expect.stringContaining("recovered; 1 session event(s)"));
    await expect(client.ingest(event("two"))).resolves.toBe(true);

    // The healed client starts over: the next outage buys three rebuilds of its own before it stops.
    healthy = false;
    await expect(client.ingest(event("three"))).resolves.toBe(false);
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

describe("the end of a native host's run", () => {
  it("with the experimental digest on, delivers, only when the run ends, the digest of a steer whose edit was checked after the step that made it", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-native-yield-")));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    vi.stubEnv("RERAIL_EXPERIMENTAL_MISSED_STEER_DIGEST", "1");
    setUpTestRepository(root, { library: [{
      kind: "code",
      rule: codeRuleSchema.parse({
        format: "rerail.policy/v1", id: "test:needle", revision: 1, mode: "steer",
        scope: { include: ["**/*.py"], exclude: [] },
        clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No needle" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
      }),
    }] });
    const daemon = await startNativeTestDaemon(root);
    // The edit is checked only once the test releases it, after its tool result has been answered.
    const release = Promise.withResolvers<void>();
    const checked = Promise.withResolvers<void>();
    const finish = CodeRuleBridge.prototype.finish;
    vi.spyOn(CodeRuleBridge.prototype, "finish").mockImplementationOnce(function (this: CodeRuleBridge, toolCallId: string, failed?: boolean) {
      void release.promise.then(async () => {
        await finish.call(this, toolCallId, failed);
        await this.settleGeneratedEdits();
        checked.resolve();
      });
      return Promise.resolve([]);
    });
    const received: AdapterDelivery[] = [];
    const client = new NativeSessionClient({
      instance: { workspaceRoot: root, url: new URL(daemon.url), accessToken: daemon.state().accessToken! },
      token: daemon.state().nativeToken!,
      host: "omp", sessionId: "yield", entries: () => [],
      appendEntry() {},
      deliver: (_client, deliveries) => { received.push(...deliveries); },
    });
    const event = <Event extends object>(value: Event) => ({ ...value, sessionId: "yield" });
    try {
      await client.initialize({ cwd: root, modelPath: resolve("model") });
      await client.activate();
      await client.awaitCodeRules();
      await client.ingest(event({ kind: "call" as const, eventId: "call", toolCallId: "write", toolName: "write", input: { path: "edited.py" } }));
      await writeFile(join(root, "edited.py"), "NEEDLE\n");
      await client.ingest(event({ kind: "result" as const, eventId: "result", toolCallId: "write", toolName: "write", output: "written" }));
      release.resolve();
      await checked.promise;

      // A later step and the turn's end pass while the steer is held: their replies collect what a poll would.
      await client.ingest(event({ kind: "user-prompt" as const, eventId: "next", text: "next" }));
      await client.ingest(event({ kind: "turn-end" as const, eventId: "turn", text: "done" }));
      expect(received).toEqual([]);

      await client.yieldDeliveries();
      expect(received).toEqual([expect.objectContaining({ requested: "steer", ruleIds: ["test:needle"], rules: [expect.objectContaining({ id: "test:needle", state: "steered" })] })]);
      expect(received[0]!.content).toMatch(/^Rerail finished checking your earlier edits after you moved on:\n/u);
      expect(received[0]!.content).toContain("- edited.py:1:1 — No needle [test:needle]");
      await client.yieldDeliveries();
      expect(received).toHaveLength(1);
    } finally {
      await client.close();
    }
  }, 30_000);
});
