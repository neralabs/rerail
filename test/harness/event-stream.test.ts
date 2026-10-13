import { afterEach, describe, expect, it, vi } from "vitest";
const routing = vi.hoisted(() => ({
  isMainRerailBuild: vi.fn<() => boolean>(),
  daemonEngineSocket: vi.fn<(replaceOtherPackage: boolean) => Promise<string>>(),
}));
vi.mock("../../src/engine/native/rerail-build.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../../src/engine/native/rerail-build.js"),
  isMainRerailBuild: routing.isMainRerailBuild,
}));
vi.mock("../../src/runtime/daemon-control.js", async () => ({
  ...await vi.importActual<Record<string, unknown>>("../../src/runtime/daemon-control.js"),
  daemonEngineSocket: routing.daemonEngineSocket,
}));
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { createServer, type Server } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough, Readable } from "node:stream";
import type { AdapterDelivery, AdapterEvent, RerailEventLoop } from "../../src/engine/index.js";
import { steerBlock } from "../../src/engine/index.js";
import { encodeRerailEventResult, RERAIL_EVENT_PROTOCOL_PREFACE } from "../../src/runtime/index.js";
import { runRerailEventStream, serveRerailEventStream } from "../../src/harness/event-stream.js";
import { startTestRepository } from "../support/started-repository.js";

const cleanups: Array<() => Promise<void>> = [];

afterEach(async () => {
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
  routing.isMainRerailBuild.mockReset();
  routing.daemonEngineSocket.mockReset();
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

  it("rejects an incompatible preface without ingesting", async () => {
    const ingest = vi.fn<(event: AdapterEvent) => Promise<readonly []>>().mockResolvedValue([]);
    expect(await runStream(["RERAIL-EVENTS/2"], { ingest })).toEqual([
      expect.objectContaining({ eventId: null, error: expect.objectContaining({ code: "unsupported-version" }) }),
    ]);
    expect(ingest).not.toHaveBeenCalled();
  });

  it("links a steer's Why? to the session checkout's dashboard, recorded in the Rerail home's daemon.json", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-event-stream-home-"));
    cleanups.push(() => rm(home, { recursive: true, force: true }));
    const checkout = join(home, "checkout");
    await mkdir(checkout);
    await writeFile(join(home, "daemon.json"), JSON.stringify({ pid: process.pid, host: "127.0.0.1", port: 4791, url: "http://127.0.0.1:4791/",
      version: "test", databasePath: join(home, "rerail.sqlite"), kernelSocketPath: join(home, "kernel.sock"), ui: "bundled", startedAt: 1, packageRoot: home }));
    vi.stubEnv("RERAIL_HOME", home);
    cleanups.push(async () => { vi.unstubAllEnvs(); });
    const steer: AdapterDelivery = { sessionId: "s1", requested: "steer", mechanism: "additional-context", content: steerBlock("acme:no-print", "Remove print.", { mode: "steer" }), ruleIds: ["acme:no-print"] };
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
      content: steer.content.replace("Why? → #rule/", "Why? → http://127.0.0.1:4791/#rule/"),
    }] });
  });
});

interface EngineRequestBody {
  event: { eventId: string };
  configuration: Record<string, string>;
}

/** Stands in for the main daemon's engine service socket, answering every event with a delivery only it produces. */
async function fakeEngineService(socketPath: string): Promise<EngineRequestBody[]> {
  const received: EngineRequestBody[] = [];
  const server: Server = createServer(async (request, response) => {
    let body = "";
    for await (const chunk of request) body += String(chunk);
    const parsed = JSON.parse(body) as EngineRequestBody;
    received.push(parsed);
    response.end(encodeRerailEventResult({
      eventId: parsed.event.eventId,
      deliveries: [{ sessionId: "s", requested: "steer", mechanism: "native-queue", content: "from the daemon", ruleIds: [] }],
    }));
  });
  await new Promise<void>(resolve => server.listen(socketPath, resolve));
  cleanups.push(async () => await new Promise<void>(resolve => server.close(() => resolve())));
  return received;
}

describe("rerail events in a main build", () => {
  it("sends every event to the daemon engine service instead of evaluating in-process", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-main-"));
    cleanups.push(async () => await rm(root, { recursive: true, force: true }));
    const socketPath = join(root, "engine.sock");
    const received = await fakeEngineService(socketPath);
    routing.isMainRerailBuild.mockReturnValue(true);
    routing.daemonEngineSocket.mockResolvedValue(socketPath);
    startTestRepository(root, join(root, "rerail.sqlite"));
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
        `${JSON.stringify({ event: { kind: "user-prompt", eventId: "prompt", sessionId: "s", text: "go" } })}\n`,
      ]),
      output,
    });

    const results = written.trim().split("\n").map(line => JSON.parse(line) as { eventId: string; deliveries: Array<{ content: string }> });
    expect(results.map(result => [result.eventId, result.deliveries.map(delivery => delivery.content)])).toEqual([
      ["start", ["from the daemon"]],
      ["prompt", ["from the daemon"]],
    ]);
    expect(received.map(request => request.event.eventId)).toEqual(["start", "prompt"]);
    expect(received[0]?.configuration).toEqual(expect.objectContaining({
      storageRoot: join(root, "sessions"),
      databasePath: join(root, "rerail.sqlite"),
    }));
    // Only the first event may replace a daemon from another package; a stream that outlives an update must
    // keep using the newer daemon rather than bring its own version back.
    expect(routing.daemonEngineSocket.mock.calls).toEqual([[true], [false]]);
  });

  it("never asks for the daemon, which would autostart it, while Rerail is off in the checkout", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-events-off-"));
    cleanups.push(async () => await rm(root, { recursive: true, force: true }));
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
    expect(routing.daemonEngineSocket).not.toHaveBeenCalled();
  });
});

