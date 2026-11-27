import { afterEach, describe, expect, it, vi } from "vitest";
const routing = vi.hoisted(() => ({
  isMainRerailBuild: vi.fn<() => boolean>(),
  daemonEngineEndpoint: vi.fn<(replaceOtherPackage: boolean, deadline?: number) => Promise<{ socketPath: string; accessToken?: string }>>(),
  engineEndpointAt: vi.fn<(socketPath: string, deadline?: number) => Promise<{ socketPath: string; accessToken?: string }>>(),
  runningDaemonEngineEndpoint: vi.fn<(deadline?: number, machine?: unknown, replaceOtherPackage?: boolean) => Promise<unknown>>(),
  // A start launched in the background would be a real `rerail daemon start`: the cases count the launches instead.
  launchAutostart: vi.fn<() => boolean>(() => true),
}));
vi.mock("../../src/engine/native/rerail-build.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../../src/engine/native/rerail-build.js"),
  isMainRerailBuild: routing.isMainRerailBuild,
}));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  routing.engineEndpointAt.mockImplementation((socketPath, deadline) => actual.engineEndpointAt(socketPath, deadline));
  routing.runningDaemonEngineEndpoint.mockImplementation(async (deadline, _machine, replaceOtherPackage) => await actual.runningDaemonEngineEndpoint(deadline, undefined, replaceOtherPackage));
  return {
    ...actual,
    daemonEngineEndpoint: routing.daemonEngineEndpoint,
    engineEndpointAt: routing.engineEndpointAt,
    runningDaemonEngineEndpoint: routing.runningDaemonEngineEndpoint,
    launchAutostart: routing.launchAutostart,
  };
});
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough, Readable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import type { AdapterDelivery, AdapterEvent, RerailEventLoop } from "../../src/engine/index.js";
import { editSteer } from "../support/steers.js";
import { encodeRerailEventResult, InactiveSessionError, RERAIL_EVENT_PROTOCOL_PREFACE } from "../../src/runtime/index.js";
import { runRerailEventStream, serveRerailEventStream } from "../../src/harness/event-stream.js";
import { HOOK_FAILURES_FILE, readHookFailures } from "../../src/harness/hook-failures.js";
import { testEngineEndpoint } from "../support/platform/engine-endpoint.js";
import { startTestRepository } from "../support/started-repository.js";
import { countingDaemon } from "../support/hook-events.js";
import { removeFolder } from "../support/platform/remove-folder.js";

// The wait the stream takes, read where it is defined: the runtime layer's index does not export it.
const { HOOK_DAEMON_WAIT_MS } = await vi.importActual<typeof import("../../src/runtime/daemon/daemon-waits.js")>("../../src/runtime/daemon/daemon-waits.js");
const cleanups: Array<() => Promise<void>> = [];

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
  routing.isMainRerailBuild.mockReset();
  routing.daemonEngineEndpoint.mockReset();
  routing.runningDaemonEngineEndpoint.mockClear();
  routing.launchAutostart.mockClear();
});

async function runStream(lines: readonly string[], eventLoop: RerailEventLoop, workspaceRoot?: string): Promise<readonly Record<string, unknown>[]> {
  const output = new PassThrough();
  const chunks: Buffer[] = [];
  output.on("data", chunk => chunks.push(Buffer.from(chunk)));
  await serveRerailEventStream({
    eventLoop,
    input: Readable.from(lines.map(line => `${line}\n`)),
    output,
    workspaceRoot,
  });
  return Buffer.concat(chunks).toString("utf8").trim().split("\n").filter(Boolean)
    .map(line => JSON.parse(line) as Record<string, unknown>);
}

describe("Rerail event stream", () => {
  it("processes ordered frames after one protocol preface", async () => {
    const ingest = vi.fn<(event: AdapterEvent) => Promise<readonly []>>().mockResolvedValue([]);
    const first = { event: { kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "s1", cwd: "/workspace" } } };
    const second = { event: { kind: "user-prompt", eventId: "prompt", sessionId: "s1", text: "hello" } };

    expect(await runStream([
      RERAIL_EVENT_PROTOCOL_PREFACE,
      JSON.stringify(first),
      JSON.stringify(second),
    ], { ingest })).toEqual([
      { eventId: "start", deliveries: [] },
      { eventId: "prompt", deliveries: [] },
    ]);
    expect(ingest.mock.calls.map(([event]) => event.eventId)).toEqual(["start", "prompt"]);
  });

  it("contains a malformed frame and continues", async () => {
    const ingest = vi.fn<(event: AdapterEvent) => Promise<readonly []>>().mockResolvedValue([]);
    const valid = { event: { kind: "session-start", eventId: "start", session: { host: "codex", sessionId: "s1", cwd: "/workspace" } } };

    expect(await runStream([
      RERAIL_EVENT_PROTOCOL_PREFACE,
      "not-json",
      JSON.stringify(valid),
    ], { ingest })).toEqual([
      expect.objectContaining({ eventId: null, error: expect.objectContaining({ code: "invalid-event" }) }),
      { eventId: "start", deliveries: [] },
    ]);
    expect(ingest).toHaveBeenCalledOnce();
  });

  it("answers an event for an inactive session by its code, and any other failure as an invalid event", async () => {
    const line = JSON.stringify({ event: { kind: "user-prompt", eventId: "prompt", sessionId: "s1", text: "hello" } });
    const answers = async (error: Error) => await runStream([RERAIL_EVENT_PROTOCOL_PREFACE, line], { ingest: async () => { throw error; } });

    expect(await answers(new InactiveSessionError("s1"))).toEqual([
      { eventId: "prompt", error: { code: "inactive-session", message: "Inactive session: s1" } },
    ]);
    // Another copy of the class, as the bundled stream sees one thrown from the package's other entry points.
    expect(await answers(Object.assign(new Error("Session s1 has ended"), { code: "inactive-session" }))).toEqual([
      { eventId: "prompt", error: { code: "inactive-session", message: "Session s1 has ended" } },
    ]);
    expect(await answers(new Error("Inactive session: s1"))).toEqual([
      { eventId: "prompt", error: { code: "invalid-event", message: "Inactive session: s1" } },
    ]);
    // A code that is not the protocol's, as a system call's error carries, is no frame's code.
    expect(await answers(Object.assign(new Error("ENOENT: no such file or directory"), { code: "ENOENT" }))).toEqual([
      { eventId: "prompt", error: { code: "invalid-event", message: "ENOENT: no such file or directory" } },
    ]);
  });

  it("rejects an incompatible preface without ingesting", async () => {
    const ingest = vi.fn<(event: AdapterEvent) => Promise<readonly []>>().mockResolvedValue([]);
    expect(await runStream(["RERAIL-EVENTS/2"], { ingest })).toEqual([
      expect.objectContaining({ eventId: null, error: expect.objectContaining({ code: "unsupported-version" }) }),
    ]);
    expect(ingest).not.toHaveBeenCalled();
  });

  it("links a steer's rules to the session checkout's dashboard, recorded in the Rerail home's daemon.json", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-event-stream-home-"));
    cleanups.push(() => removeFolder(home));
    const checkout = join(home, "checkout");
    await mkdir(checkout);
    await writeFile(join(home, "daemon.json"), JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 4791, url: "http://127.0.0.1:4791/",
      version: "test", databasePath: join(home, "rerail.sqlite"), kernelSocketPath: testEngineEndpoint(home), ui: "bundled", startedAt: 1, packageRoot: home }));
    vi.stubEnv("RERAIL_HOME", home);
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    const steer: AdapterDelivery = { sessionId: "s1", requested: "steer", mechanism: "additional-context", content: editSteer("acme:no-print", "Remove print."), ruleIds: ["acme:no-print"], rules: [{ id: "acme:no-print", state: "steered" }] };
    const ingest = vi.fn<(event: AdapterEvent) => Promise<readonly AdapterDelivery[]>>()
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([steer]);

    const frames = await runStream([
      RERAIL_EVENT_PROTOCOL_PREFACE,
      JSON.stringify({ event: { kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "s1", cwd: checkout } } }),
      JSON.stringify({ event: { kind: "result", eventId: "result", sessionId: "s1", toolCallId: "t1", toolName: "Write", output: "ok" } }),
    ], { ingest }, "/elsewhere");
    expect(frames[1]).toEqual({ eventId: "result", deliveries: [{
      ...steer,
      // The agent's text is the runtime's own; only the rules carry the link.
      rules: [{ id: "acme:no-print", state: "steered", url: "http://127.0.0.1:4791/#rule/acme%3Ano-print" }],
    }] });
  });
});

interface EngineRequestBody {
  event: { eventId: string };
  configuration: Record<string, string>;
  /** The `x-rerail-access` header the request carried. */
  access: string | undefined;
}

/** Stands in for the main daemon's engine service socket, answering every event with a delivery only it produces. */
async function fakeEngineService(socketPath: string): Promise<EngineRequestBody[]> {
  const received: EngineRequestBody[] = [];
  const server: Server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += String(chunk);
    const parsed = JSON.parse(body) as Omit<EngineRequestBody, "access">;
    const access = request.headers["x-rerail-access"];
    received.push({ ...parsed, access: typeof access === "string" ? access : undefined });
    response.end(encodeRerailEventResult({
      eventId: parsed.event.eventId,
      deliveries: [{ sessionId: "s", requested: "steer", mechanism: "native-queue", content: "from the daemon", ruleIds: [], rules: [] }],
    }));
  });
  await new Promise<void>(resolve => server.listen(socketPath, resolve));
  cleanups.push(async () => await new Promise<void>(resolve => server.close(() => resolve())));
  return received;
}

/** Stands in for a daemon's engine service that refuses each event in turn with HTTP 400 and the body `refusals` gives. */
async function refusingEngineService(socketPath: string, refusals: readonly string[]): Promise<void> {
  const left = [...refusals];
  const server: Server = createServer((request, response) => {
    request.resume();
    request.once("end", () => response.writeHead(400).end(left.shift()));
  });
  await new Promise<void>(resolve => server.listen(socketPath, resolve));
  cleanups.push(async () => await new Promise<void>(resolve => server.close(() => resolve())));
}

describe("rerail events in a main build", () => {
  it("sends every event to the daemon engine service instead of evaluating in-process", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-main-"));
    cleanups.push(async () => await removeFolder(root));
    const socketPath = testEngineEndpoint(root);
    const received = await fakeEngineService(socketPath);
    routing.isMainRerailBuild.mockReturnValue(true);
    routing.daemonEngineEndpoint.mockResolvedValue({ socketPath, accessToken: "daemon-capability" });
    startTestRepository(root, join(root, "rerail.sqlite"));
    const output = new PassThrough();
    let written = "";
    output.setEncoding("utf8").on("data", (chunk: string) => { written += chunk; });
    const session = { host: "claude-code", sessionId: "s", cwd: root };

    const before = Date.now();
    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([
        `${RERAIL_EVENT_PROTOCOL_PREFACE}\n`,
        `${JSON.stringify({ event: { kind: "session-start", eventId: "start", session } })}\n`,
        `${JSON.stringify({ event: { kind: "user-prompt", eventId: "prompt", sessionId: "s", text: "go" } })}\n`,
      ]),
      output,
    });
    const after = Date.now();

    const results = written.trim().split("\n").map(line => JSON.parse(line) as { eventId: string; deliveries: Array<{ content: string }> });
    expect(results.map(result => [result.eventId, result.deliveries.map(delivery => delivery.content)])).toEqual([
      ["start", ["from the daemon"]],
      ["prompt", ["from the daemon"]],
    ]);
    expect(received.map(request => request.event.eventId)).toEqual(["start", "prompt"]);
    // The daemon's engine socket refuses a request without its capability.
    expect(received.map(request => request.access)).toEqual(["daemon-capability", "daemon-capability"]);
    expect(received[0]?.configuration).toEqual(expect.objectContaining({
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
    }));
    // Only the first event may replace a daemon from another package; a stream that outlives an update must
    // keep using the newer daemon rather than bring its own version back.
    expect(routing.daemonEngineEndpoint.mock.calls.map(([replaceOtherPackage]) => replaceOtherPackage)).toEqual([true, false]);
    // Each event waits for a busy daemon until 12 s after it arrived, the start check's read of the daemon included, as
    // a hook does; without a deadline it waited 2 s for the start check, then 2 s and 10 s for the endpoint.
    for (const [, deadline] of routing.daemonEngineEndpoint.mock.calls) {
      expect(deadline).toBeGreaterThanOrEqual(before + HOOK_DAEMON_WAIT_MS);
      expect(deadline).toBeLessThanOrEqual(after + HOOK_DAEMON_WAIT_MS);
    }
  });

  // The daemon refuses an event of a session it does not hold with the protocol's code beside its runtime's message; a
  // daemon older than that code sends the message alone, and its refusal reads as before.
  it("answers an event the daemon refused by the code and message its answer carries, and an older daemon's refusal as an invalid event", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-refused-"));
    cleanups.push(async () => await removeFolder(root));
    const socketPath = testEngineEndpoint(root);
    const older = JSON.stringify({ error: "Inactive session: ended" });
    await refusingEngineService(socketPath, [JSON.stringify({ error: "Inactive session: never-started", code: "inactive-session" }), older]);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", "0");
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    routing.isMainRerailBuild.mockReturnValue(true);
    routing.daemonEngineEndpoint.mockResolvedValue({ socketPath });
    startTestRepository(root, join(root, "rerail.sqlite"));
    const prompt = (eventId: string, sessionId: string): string => JSON.stringify({ event: { kind: "user-prompt", eventId, sessionId, text: "go" } });
    const output = new PassThrough();
    let written = "";
    output.setEncoding("utf8").on("data", (chunk: string) => { written += chunk; });

    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([RERAIL_EVENT_PROTOCOL_PREFACE, prompt("never", "never-started"), prompt("ended", "ended")].map(line => `${line}\n`)),
      output,
    });

    expect(written.trim().split("\n").map(line => JSON.parse(line) as unknown)).toEqual([
      { eventId: "never", error: { code: "inactive-session", message: "Inactive session: never-started" } },
      { eventId: "ended", error: { code: "invalid-event", message: `Kernel request failed (400): ${older}` } },
    ]);
  });

  /** A session's start and nine prompts, as a coding agent's stream sends them. */
  function sessionOfTen(cwd: string): Array<Record<string, unknown> & { eventId: string }> {
    const session = { host: "claude-code", sessionId: "s", cwd };
    return [
      { kind: "session-start", eventId: "s:start", session },
      ...Array.from({ length: 9 }, (_, index) => ({ kind: "user-prompt", eventId: `s:prompt:${index + 1}`, sessionId: "s", session, text: `Prompt ${index + 1}` })),
    ];
  }

  // Beside another process's write lock on the database every daemon start fails, after the wait a start takes. Only
  // the session's start waits for one, as a hook's does: each later event reads the daemon's record, finds no daemon
  // running, and is answered at once, while a start for the next event is launched in the background.
  it("waits for a daemon start only at a session's start, answers each later event at once while none runs, and records each event lost", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-no-daemon-"));
    cleanups.push(async () => await removeFolder(root));
    const home = join(root, "home");
    vi.stubEnv("RERAIL_HOME", home);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    // A person's shell, where an event may start the daemon; the test run turns that off everywhere else.
    vi.stubEnv("RERAIL_AUTOSTART", "1");
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    routing.isMainRerailBuild.mockReturnValue(true);
    const failedStart = "Rerail daemon exited with code 1 before it was ready: rerail daemon: Failed to run the query 'BEGIN IMMEDIATE'";
    routing.daemonEngineEndpoint.mockImplementation(async () => {
      await delay(2_000);
      throw new Error(failedStart);
    });
    startTestRepository(root, join(root, "rerail.sqlite"));
    const events = sessionOfTen(root);
    const output = new PassThrough();
    let written = "";
    const answeredAt: number[] = [];
    output.setEncoding("utf8").on("data", (chunk: string) => {
      written += chunk;
      answeredAt.push(...Array.from(chunk.matchAll(/\n/gu), () => Date.now()));
    });

    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([RERAIL_EVENT_PROTOCOL_PREFACE, ...events.map(event => JSON.stringify({ event }))].map(line => `${line}\n`)),
      output,
    });

    // The stream reads each frame once the one before is answered, so an answer's distance from the one before is how
    // long its event waited.
    const waits = answeredAt.slice(1).map((at, index) => at - answeredAt[index]!);
    expect.soft(waits).toHaveLength(9);
    expect.soft(Math.max(...waits)).toBeLessThan(1_000);
    const lost = "no Rerail daemon answered, so this event was not recorded";
    // Sound events no daemon took: the code tells them apart from a frame the stream could not read.
    expect.soft(written.trim().split("\n").map(line => JSON.parse(line) as unknown)).toEqual([
      { eventId: "s:start", error: { code: "internal-error", message: failedStart } },
      ...events.slice(1).map(({ eventId }) => ({ eventId, error: { code: "internal-error", message: lost } })),
    ]);
    // Where `rerail status` looks, as for a hook's event: the start whose daemon failed, then each prompt.
    expect.soft(readHookFailures(home, 0).failures.map(({ host, event, reason }) => ({ host, event, reason }))).toEqual([
      { host: "claude-code", event: "session-start", reason: failedStart },
      ...Array.from({ length: 9 }, () => ({ host: "claude-code", event: "user-prompt", reason: lost })),
    ]);
    expect.soft(routing.daemonEngineEndpoint).toHaveBeenCalledOnce();
    // Each prompt that found none asks for a start for the next event; the home's claim keeps it to one launch at a time.
    expect(routing.launchAutostart).toHaveBeenCalledTimes(9);
  }, 60_000);

  it("sends a later event to the daemon that runs, which it neither starts nor, after the stream's first event, replaces", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-running-"));
    cleanups.push(async () => await removeFolder(root));
    const socketPath = testEngineEndpoint(root);
    const received = await fakeEngineService(socketPath);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", "1");
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    routing.isMainRerailBuild.mockReturnValue(true);
    routing.daemonEngineEndpoint.mockResolvedValue({ socketPath, accessToken: "daemon-capability" });
    routing.runningDaemonEngineEndpoint.mockResolvedValueOnce({ status: "running", endpoint: { socketPath, accessToken: "daemon-capability" } });
    startTestRepository(root, join(root, "rerail.sqlite"));
    const [start, prompt] = sessionOfTen(root);

    const before = Date.now();
    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([RERAIL_EVENT_PROTOCOL_PREFACE, JSON.stringify({ event: start }), JSON.stringify({ event: prompt })].map(line => `${line}\n`)),
      output: new PassThrough(),
    });
    const after = Date.now();

    expect(received.map(request => request.event.eventId)).toEqual(["s:start", "s:prompt:1"]);
    expect(routing.daemonEngineEndpoint.mock.calls.map(([replaceOtherPackage]) => replaceOtherPackage)).toEqual([true]);
    // A stream that outlives `rerail update` keeps the newer daemon: only its first event may replace another package's.
    expect(routing.runningDaemonEngineEndpoint.mock.calls.map(([, machine, replaceOtherPackage]) => [machine, replaceOtherPackage])).toEqual([[undefined, false]]);
    const [deadline] = routing.runningDaemonEngineEndpoint.mock.calls[0]!;
    expect(deadline).toBeGreaterThanOrEqual(before + HOOK_DAEMON_WAIT_MS);
    expect(deadline).toBeLessThanOrEqual(after + HOOK_DAEMON_WAIT_MS);
    expect(routing.launchAutostart).not.toHaveBeenCalled();
    expect(existsSync(join(root, "home", HOOK_FAILURES_FILE))).toBe(false);
  });

  // A daemon can stop between the lookup and the send (a crash, `rerail daemon stop`): as a hook reads it, nothing
  // listens at its socket, so its event is one no daemon took. An event the daemon refuses with no code stays invalid.
  it("answers an event whose daemon is gone at the send as one no daemon took, and one the daemon refuses as invalid", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-gone-"));
    cleanups.push(async () => await removeFolder(root));
    const home = join(root, "home");
    vi.stubEnv("RERAIL_HOME", home);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", "1");
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    routing.isMainRerailBuild.mockReturnValue(true);
    routing.daemonEngineEndpoint.mockResolvedValue({ socketPath: testEngineEndpoint(home), accessToken: "daemon-capability" });
    const refusing = testEngineEndpoint(root);
    await refusingEngineService(refusing, [JSON.stringify({ error: "refused" })]);
    routing.runningDaemonEngineEndpoint.mockResolvedValueOnce({ status: "running", endpoint: { socketPath: refusing, accessToken: "daemon-capability" } });
    startTestRepository(root, join(root, "rerail.sqlite"));
    const [start, prompt] = sessionOfTen(root);
    const output = new PassThrough();
    let written = "";
    output.setEncoding("utf8").on("data", (chunk: string) => { written += chunk; });

    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([RERAIL_EVENT_PROTOCOL_PREFACE, JSON.stringify({ event: start }), JSON.stringify({ event: prompt })].map(line => `${line}\n`)),
      output,
    });

    expect.soft(written.trim().split("\n").map(line => JSON.parse(line) as unknown)).toEqual([
      { eventId: "s:start", error: { code: "internal-error", message: expect.stringContaining("ENOENT") } },
      { eventId: "s:prompt:1", error: { code: "invalid-event", message: 'Kernel request failed (400): {"error":"refused"}' } },
    ]);
    expect.soft(readHookFailures(home, 0).failures.map(({ event }) => event)).toEqual(["session-start", "user-prompt"]);
    // A start for the next event, as when the lookup finds no daemon; none for the daemon that answered.
    expect(routing.launchAutostart).toHaveBeenCalledOnce();
  });

  // A start brings up the home's daemon, not the engine a socket named outright stands for: as at its lookup, none starts.
  it("launches no start when nothing listens at the socket RERAIL_ENGINE_SOCKET names", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-named-gone-"));
    cleanups.push(async () => await removeFolder(root));
    const named = testEngineEndpoint(join(root, "gone"));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    vi.stubEnv("RERAIL_ENGINE_SOCKET", named);
    vi.stubEnv("RERAIL_AUTOSTART", "1");
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    routing.isMainRerailBuild.mockReturnValue(true);
    routing.engineEndpointAt.mockResolvedValueOnce({ socketPath: named });
    startTestRepository(root, join(root, "rerail.sqlite"));
    const [start] = sessionOfTen(root);
    const output = new PassThrough();
    let written = "";
    output.setEncoding("utf8").on("data", (chunk: string) => { written += chunk; });

    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([RERAIL_EVENT_PROTOCOL_PREFACE, JSON.stringify({ event: start })].map(line => `${line}\n`)),
      output,
    });

    expect.soft(JSON.parse(written) as unknown).toEqual({ eventId: "s:start", error: { code: "internal-error", message: expect.stringContaining("ENOENT") } });
    expect(routing.launchAutostart).not.toHaveBeenCalled();
  });

  // One stream may carry several sessions, each of its own coding agent.
  it("writes an event no daemon took under its own session's coding agent", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-agents-"));
    cleanups.push(async () => await removeFolder(root));
    const home = join(root, "home");
    vi.stubEnv("RERAIL_HOME", home);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", "1");
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    routing.isMainRerailBuild.mockReturnValue(true);
    routing.daemonEngineEndpoint.mockRejectedValue(new Error("Rerail daemon exited with code 1 before it was ready"));
    startTestRepository(root, join(root, "rerail.sqlite"));
    const [start] = sessionOfTen(root);
    const codex = { kind: "user-prompt", eventId: "c:prompt", sessionId: "c", session: { host: "codex", sessionId: "c", cwd: root }, text: "go" };

    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([RERAIL_EVENT_PROTOCOL_PREFACE, JSON.stringify({ event: start }), JSON.stringify({ event: codex })].map(line => `${line}\n`)),
      output: new PassThrough(),
    });

    expect(readHookFailures(home, 0).failures.map(({ host, event }) => [host, event])).toEqual([
      ["claude-code", "session-start"],
      ["codex", "user-prompt"],
    ]);
  });

  // No event starts a daemon under RERAIL_AUTOSTART=0, so none waits for a start: each asks as on a session's start, and
  // finding no daemon is what the person asked for, which `rerail status` says itself.
  it("asks for the daemon at each event under RERAIL_AUTOSTART=0, starts none and records nothing", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-autostart-off-"));
    cleanups.push(async () => await removeFolder(root));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    vi.stubEnv("RERAIL_ENGINE_SOCKET", undefined);
    vi.stubEnv("RERAIL_AUTOSTART", "0");
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    routing.isMainRerailBuild.mockReturnValue(true);
    const { AutostartOffError } = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
    const off = new AutostartOffError("RERAIL_AUTOSTART=0 requires a running daemon; run rerail daemon start or set RERAIL_ENGINE_SOCKET");
    routing.daemonEngineEndpoint.mockRejectedValue(off);
    startTestRepository(root, join(root, "rerail.sqlite"));
    const [start, prompt] = sessionOfTen(root);
    const output = new PassThrough();
    let written = "";
    output.setEncoding("utf8").on("data", (chunk: string) => { written += chunk; });

    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([RERAIL_EVENT_PROTOCOL_PREFACE, JSON.stringify({ event: start }), JSON.stringify({ event: prompt })].map(line => `${line}\n`)),
      output,
    });

    expect(written.trim().split("\n").map(line => JSON.parse(line) as unknown)).toEqual([
      { eventId: "s:start", error: { code: "internal-error", message: off.message } },
      { eventId: "s:prompt:1", error: { code: "internal-error", message: off.message } },
    ]);
    expect(routing.daemonEngineEndpoint).toHaveBeenCalledTimes(2);
    expect(routing.runningDaemonEngineEndpoint).not.toHaveBeenCalled();
    expect(routing.launchAutostart).not.toHaveBeenCalled();
    expect(existsSync(join(root, "home", HOOK_FAILURES_FILE))).toBe(false);
  });

  it("links a steer from the daemon's answer, asking it nothing more, and reads an older daemon's dashboard after it", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-linked-"));
    cleanups.push(async () => await removeFolder(root));
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    const daemon = await countingDaemon(join(root, "home"));
    cleanups.push(daemon.close);
    const dashboard = "http://127.0.0.1:4791/?repository=clone%3Aapp&checkout=%2Fwork%2Fapp";
    const socketPath = testEngineEndpoint(root);
    // The daemon's engine socket: every answer steers on one rule, and names the dashboard to a stream that asks for it,
    // but for the event "older", answered as a daemon older than the field does.
    const server = createServer(async (request, response) => {
      let body = "";
      for await (const chunk of request) body += String(chunk);
      const { event } = JSON.parse(body) as { event: AdapterEvent };
      const asks = request.headers["x-rerail-event-answer"] === "2" && event.eventId !== "older";
      response.end(JSON.stringify({
        eventId: event.eventId,
        deliveries: [{ sessionId: "s", requested: "steer", mechanism: "native-queue", content: editSteer("acme:no-print", "Remove print."), ruleIds: ["acme:no-print"], rules: [{ id: "acme:no-print", state: "steered" }] }],
        ...(asks ? { dashboard } : {}),
      }));
    });
    await new Promise<void>(resolve => server.listen(socketPath, resolve));
    cleanups.push(async () => await new Promise<void>(resolve => server.close(() => resolve())));
    routing.isMainRerailBuild.mockReturnValue(true);
    routing.daemonEngineEndpoint.mockResolvedValue({ socketPath, accessToken: "daemon-capability" });
    startTestRepository(root, join(root, "rerail.sqlite"));
    const output = new PassThrough();
    let written = "";
    output.setEncoding("utf8").on("data", (chunk: string) => { written += chunk; });
    await runRerailEventStream({
      workspaceRoot: root, storageRoot: join(root, "sessions"), databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([
        `${RERAIL_EVENT_PROTOCOL_PREFACE}\n`,
        `${JSON.stringify({ event: { kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "s", cwd: root } } })}\n`,
        `${JSON.stringify({ event: { kind: "user-prompt", eventId: "older", sessionId: "s", text: "go" } })}\n`,
      ]),
      output,
    });
    const links = written.trim().split("\n").map(line => (JSON.parse(line) as { deliveries: Array<{ rules: Array<{ url?: string }> }> }).deliveries[0]?.rules[0]?.url);
    expect(links).toEqual([`${dashboard}#rule/acme%3Ano-print`, `${daemon.origin}#rule/acme%3Ano-print`]);
    expect(daemon.requests).toEqual(["/api/repository"]);
  });

  it("looks the engine socket RERAIL_ENGINE_SOCKET names up with each event's deadline, as a hook does", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-named-deadline-"));
    cleanups.push(async () => await removeFolder(root));
    const socketPath = testEngineEndpoint(root);
    await fakeEngineService(socketPath);
    routing.isMainRerailBuild.mockReturnValue(true);
    routing.engineEndpointAt.mockClear();
    routing.engineEndpointAt.mockResolvedValueOnce({ socketPath }).mockResolvedValueOnce({ socketPath });
    vi.stubEnv("RERAIL_ENGINE_SOCKET", socketPath);
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    startTestRepository(root, join(root, "rerail.sqlite"));

    const before = Date.now();
    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([
        `${RERAIL_EVENT_PROTOCOL_PREFACE}\n`,
        `${JSON.stringify({ event: { kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "s", cwd: root } } })}\n`,
        `${JSON.stringify({ event: { kind: "user-prompt", eventId: "prompt", sessionId: "s", text: "go" } })}\n`,
      ]),
      output: new PassThrough(),
    });
    const after = Date.now();

    // Each event's lookup of the named socket is handed the deadline the event got when it arrived, 12 s on.
    expect(routing.engineEndpointAt.mock.calls.map(([named]) => named)).toEqual([socketPath, socketPath]);
    for (const [, deadline] of routing.engineEndpointAt.mock.calls) {
      expect(deadline).toBeGreaterThanOrEqual(before + HOOK_DAEMON_WAIT_MS);
      expect(deadline).toBeLessThanOrEqual(after + HOOK_DAEMON_WAIT_MS);
    }
  });

  // Off Windows the daemon's socket sits in a folder only the person can open, so no other account can hold it.
  it.runIf(process.platform === "win32")("sends nothing, not even a proof's challenge, to the engine pipe RERAIL_ENGINE_SOCKET names when Windows names a holder that is not this home's daemon", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-named-"));
    cleanups.push(async () => await removeFolder(root));
    // Another process listens on the pipe daemon.json names, as after a logoff or a reboot ended the daemon that
    // wrote it: this process, which Windows names as the holder, started after the start daemon.json records.
    const socketPath = testEngineEndpoint(root);
    const received: string[] = [];
    const server = createServer((request, response) => {
      request.resume();
      received.push(`${request.method} ${request.url} ${String(request.headers["x-rerail-access"] ?? "no capability")}`);
      response.writeHead(500).end();
    });
    await new Promise<void>(resolve => server.listen(socketPath, resolve));
    cleanups.push(async () => await new Promise<void>(resolve => server.close(() => resolve())));
    await writeFile(join(root, "daemon.json"), JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 1, url: "http://127.0.0.1:1/", version: "test",
      databasePath: join(root, "rerail.sqlite"), kernelSocketPath: socketPath, ui: "bundled", startedAt: 1, packageRoot: root, accessToken: "daemon-capability" }));
    vi.stubEnv("RERAIL_HOME", root);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", socketPath);
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    routing.isMainRerailBuild.mockReturnValue(true);
    startTestRepository(root, join(root, "rerail.sqlite"));
    const output = new PassThrough();
    let written = "";
    output.setEncoding("utf8").on("data", (chunk: string) => { written += chunk; });

    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([
        `${RERAIL_EVENT_PROTOCOL_PREFACE}\n`,
        `${JSON.stringify({ event: { kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "s", cwd: root } } })}\n`,
        `${JSON.stringify({ event: { kind: "user-prompt", eventId: "prompt", sessionId: "s", text: "go" } })}\n`,
      ]),
      output,
    });

    // Nothing reached the pipe: no challenge, no capability and no event.
    expect(received).toEqual([]);
    const notThisHome = { code: "internal-error", message: expect.stringContaining(`${socketPath} is not this Rerail home's daemon`) };
    expect(written.trim().split("\n").map(line => JSON.parse(line) as unknown)).toEqual([
      { eventId: "start", error: notThisHome },
      { eventId: "prompt", error: notThisHome },
    ]);
    expect(routing.daemonEngineEndpoint).not.toHaveBeenCalled();
  });

  it.runIf(process.platform === "win32")("sends the events, with no capability, to the daemon on the pipe RERAIL_ENGINE_SOCKET names when its daemon.json records none, as a daemon before Rerail 0.1.1 wrote it", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-no-capability-"));
    cleanups.push(async () => await removeFolder(root));
    // This process serves the pipe as the daemon daemon.json records, which stamped its start after the process began.
    const socketPath = testEngineEndpoint(root);
    const received = await fakeEngineService(socketPath);
    await writeFile(join(root, "daemon.json"), JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 1, url: "http://127.0.0.1:1/", version: "0.1.0",
      databasePath: join(root, "rerail.sqlite"), kernelSocketPath: socketPath, ui: "bundled", startedAt: Date.now(), packageRoot: root }));
    vi.stubEnv("RERAIL_HOME", root);
    vi.stubEnv("RERAIL_ENGINE_SOCKET", socketPath);
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    routing.isMainRerailBuild.mockReturnValue(true);
    startTestRepository(root, join(root, "rerail.sqlite"));

    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([
        `${RERAIL_EVENT_PROTOCOL_PREFACE}\n`,
        `${JSON.stringify({ event: { kind: "session-start", eventId: "start", session: { host: "claude-code", sessionId: "s", cwd: root } } })}\n`,
        `${JSON.stringify({ event: { kind: "user-prompt", eventId: "prompt", sessionId: "s", text: "go" } })}\n`,
      ]),
      output: new PassThrough(),
    });

    expect(received.map(({ event, access }) => [event.eventId, access])).toEqual([["start", undefined], ["prompt", undefined]]);
  });

  it("never asks for the daemon, which would autostart it, while Rerail is off in the checkout", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-off-"));
    cleanups.push(async () => await removeFolder(root));
    routing.isMainRerailBuild.mockReturnValue(true);
    const output = new PassThrough();
    let written = "";
    output.setEncoding("utf8").on("data", (chunk: string) => { written += chunk; });
    const session = { host: "claude-code", sessionId: "s", cwd: root };

    await runRerailEventStream({
      workspaceRoot: root,
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
      input: Readable.from([
        `${RERAIL_EVENT_PROTOCOL_PREFACE}\n`,
        `${JSON.stringify({ event: { kind: "session-start", eventId: "start", session } })}\n`,
      ]),
      output,
    });

    expect(JSON.parse(written.trim())).toEqual({ eventId: "start", deliveries: [] });
    expect(routing.daemonEngineEndpoint).not.toHaveBeenCalled();
  });
});

