import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough, Readable } from "node:stream";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DAEMON_STATE_FILE } from "../../src/runtime/index.js";
import { runRerailMcpBridge } from "../../src/api/mcp-bridge.js";
import { otherUserPid } from "../support/other-user-pid.js";

/** The Rerail home and which daemon pid answers on each local port; a step may change either to stage a restart. */
interface Machine {
  home: string;
  listeners: Map<number, number>;
}

type FetchStep = Response | Error | ((machine: Machine) => Promise<Response>);

interface BridgeRun {
  stdout: readonly string[];
  stderr: readonly string[];
  /** Bodies relayed to `/mcp`. */
  requests: readonly string[];
  /** `/mcp` endpoints dialled. */
  endpoints: readonly string[];
  /** `/healthz` endpoints probed. */
  probes: readonly string[];
}

interface RecordedDaemon {
  pid: number;
  port: number;
  startedAt?: number;
}

interface BridgeSetup {
  /** The daemon the home records; null records none. Defaults to a live one on port 4321. */
  daemon?: RecordedDaemon | null;
  /** The pid that answers on the recorded port; null when nothing listens there. Defaults to the recorded pid. */
  listener?: number | null;
}

const NOT_RUNNING = "Rerail daemon is not running; start it with rerail daemon start";
const REFUSED = "Rerail daemon refused the access capability in daemon.json (401); run rerail daemon stop, then rerail daemon start";

const directories: string[] = [];

afterEach(async () => {
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function jsonResponse(status: number, body: unknown, contentType = "application/json"): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": contentType } });
}

function collect(stream: PassThrough): () => string[] {
  const chunks: Buffer[] = [];
  stream.on("data", (chunk) => chunks.push(Buffer.from(chunk)));
  return () => Buffer.concat(chunks).toString("utf8").split("\n").filter(Boolean);
}

async function recordDaemon(home: string, daemon: RecordedDaemon): Promise<void> {
  await mkdir(home, { recursive: true });
  await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify({
    pid: daemon.pid,
    host: "127.0.0.1",
    port: daemon.port,
    url: `http://127.0.0.1:${daemon.port}/`,
    version: "0.0.0",
    databasePath: join(home, "rerail.sqlite"),
    kernelSocketPath: "/tmp/rerail-bridge-test.sock",
    ui: "bundled",
    startedAt: daemon.startedAt ?? 0,
    packageRoot: "/tmp/rerail-bridge-test-package",
  }));
}

/** The pid of a process that has already exited. */
function exitedPid(): number {
  const pid = spawnSync(process.execPath, ["-e", ""]).pid;
  if (pid === undefined) throw new Error("no pid for the exited process");
  return pid;
}

async function runBridge(lines: readonly string[], steps: readonly FetchStep[], setup: BridgeSetup = {}): Promise<BridgeRun> {
  const root = await mkdtemp(join(tmpdir(), "rerail-bridge-"));
  directories.push(root);
  const home = join(root, "home");
  const daemon = setup.daemon === undefined ? { pid: process.pid, port: 4321 } : setup.daemon;
  const machine: Machine = { home, listeners: new Map() };
  if (daemon !== null) {
    await recordDaemon(home, daemon);
    const listener = setup.listener === undefined ? daemon.pid : setup.listener;
    if (listener !== null) machine.listeners.set(daemon.port, listener);
  }
  const requests: string[] = [];
  const endpoints: string[] = [];
  const probes: string[] = [];
  const remaining = [...steps];
  const fetchImpl = vi.fn(async (endpoint: URL, init: RequestInit): Promise<Response> => {
    const listener = machine.listeners.get(Number(endpoint.port));
    if (endpoint.pathname === "/healthz") {
      probes.push(endpoint.href);
      if (listener === undefined) throw new TypeError("fetch failed");
      return jsonResponse(200, { status: "ok", pid: listener });
    }
    endpoints.push(endpoint.href);
    requests.push(String(init.body));
    if (listener === undefined) throw new TypeError("fetch failed");
    const step = remaining.shift();
    if (step === undefined) throw new Error("unexpected request");
    if (typeof step === "function") return await step(machine);
    if (step instanceof Error) throw step;
    return step;
  });
  const stdout = new PassThrough();
  const stderr = new PassThrough();
  const collectedStdout = collect(stdout);
  const collectedStderr = collect(stderr);
  await runRerailMcpBridge({
    input: Readable.from(lines.map((line) => `${line}\n`)),
    stdout,
    stderr,
    fetch: fetchImpl,
    home,
  });
  return { stdout: collectedStdout(), stderr: collectedStderr(), requests, endpoints, probes };
}

describe("Rerail MCP bridge", () => {
  it("relays successful responses from the machine's daemon verbatim", async () => {
    const request = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" });
    const result = JSON.stringify({ jsonrpc: "2.0", id: 1, result: {} });
    const run = await runBridge([request], [new Response(result, { status: 200 })]);
    expect(run.endpoints).toEqual(["http://127.0.0.1:4321/mcp"]);
    expect(run.requests).toEqual([request]);
    expect(run.stdout).toEqual([result]);
    expect(run.stderr).toEqual([]);
  });

  it("relays non-2xx JSON-RPC error bodies to stdout", async () => {
    const parseError = { jsonrpc: "2.0", error: { code: -32700, message: "Parse error: Invalid JSON" }, id: null };
    const run = await runBridge(["{not json"], [jsonResponse(400, parseError)]);
    expect(run.stdout).toEqual([JSON.stringify(parseError)]);
    expect(run.stderr).toEqual([]);
  });

  it("answers a failed fetch with a JSON-RPC error and keeps serving", async () => {
    const first = JSON.stringify({ jsonrpc: "2.0", id: 7, method: "tools/list" });
    const second = JSON.stringify({ jsonrpc: "2.0", id: "b", method: "ping" });
    const pong = JSON.stringify({ jsonrpc: "2.0", id: "b", result: {} });
    const run = await runBridge([first, second], [new Error("fetch failed: ECONNREFUSED"), new Response(pong, { status: 200 })]);
    expect(run.stdout).toEqual([
      JSON.stringify({ jsonrpc: "2.0", id: 7, error: { code: -32000, message: "fetch failed: ECONNREFUSED" } }),
      pong,
    ]);
    expect(run.stderr).toEqual(["rerail: mcp bridge: fetch failed: ECONNREFUSED"]);
    expect(run.requests).toEqual([first, second]);
  });

  it("answers a non-JSON-RPC failure body with an error carrying the status", async () => {
    const request = JSON.stringify({ jsonrpc: "2.0", id: 3, method: "ping" });
    const run = await runBridge([request], [new Response("Bad Gateway", { status: 502, headers: { "content-type": "text/plain" } })]);
    expect(run.stdout).toEqual([
      JSON.stringify({ jsonrpc: "2.0", id: 3, error: { code: -32000, message: "Rerail MCP bridge request failed (502): Bad Gateway" } }),
    ]);
    expect(run.stderr).toEqual(["rerail: mcp bridge: Rerail MCP bridge request failed (502): Bad Gateway"]);
  });

  it("answers every request id of a failed batch", async () => {
    const batch = JSON.stringify([
      { jsonrpc: "2.0", id: 1, method: "ping" },
      { jsonrpc: "2.0", method: "notifications/initialized" },
      { jsonrpc: "2.0", id: 2, method: "ping" },
    ]);
    const run = await runBridge([batch], [new Error("timeout")]);
    expect(run.stdout).toEqual([JSON.stringify([
      { jsonrpc: "2.0", id: 1, error: { code: -32000, message: "timeout" } },
      { jsonrpc: "2.0", id: 2, error: { code: -32000, message: "timeout" } },
    ])]);
  });

  it("only logs when a notification cannot be delivered", async () => {
    const notification = JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" });
    const request = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" });
    const pong = JSON.stringify({ jsonrpc: "2.0", id: 1, result: {} });
    const run = await runBridge([notification, request], [new Error("fetch failed"), new Response(pong, { status: 200 })]);
    expect(run.stdout).toEqual([pong]);
    expect(run.stderr).toEqual(["rerail: mcp bridge: fetch failed"]);
  });

  it("writes nothing for an accepted notification", async () => {
    const notification = JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" });
    const run = await runBridge([notification, ""], [new Response(null, { status: 202 })]);
    expect(run.stdout).toEqual([]);
    expect(run.stderr).toEqual([]);
  });

  it("sends each request to the daemon the Rerail home records, on whatever port it holds", async () => {
    const request = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" });
    const pong = JSON.stringify({ jsonrpc: "2.0", id: 1, result: {} });
    const run = await runBridge([request], [new Response(pong, { status: 200 })], { daemon: { pid: process.pid, port: 51_234 } });
    expect(run.endpoints).toEqual(["http://127.0.0.1:51234/mcp"]);
    expect(run.stdout).toEqual([pong]);
  });

  it.each([
    ["records no daemon", null],
    ["records a daemon whose process has exited", "exited"],
  ] as const)("answers with an error and dials nothing when the home %s", async (_case, recorded) => {
    const request = JSON.stringify({ jsonrpc: "2.0", id: 4, method: "tools/list" });
    const daemon = recorded === null ? null : { pid: exitedPid(), port: 51_234 };
    const run = await runBridge([request], [], { daemon });
    expect(run.requests).toEqual([]);
    expect(run.probes).toEqual([]);
    expect(run.stdout).toEqual([JSON.stringify({ jsonrpc: "2.0", id: 4, error: { code: -32000, message: NOT_RUNNING } })]);
    expect(run.stderr).toEqual([expect.stringContaining(NOT_RUNNING)]);
  });

  const foreignPid = otherUserPid();
  it.runIf(foreignPid !== undefined)("never probes a recorded pid that another OS user runs, even when its port answers as that pid", async () => {
    const request = JSON.stringify({ jsonrpc: "2.0", id: 4, method: "tools/list" });
    const run = await runBridge([request], [], { daemon: { pid: foreignPid!, port: 51_234 } });
    expect(run.probes).toEqual([]);
    expect(run.requests).toEqual([]);
    expect(run.stdout).toEqual([JSON.stringify({ jsonrpc: "2.0", id: 4, error: { code: -32000, message: NOT_RUNNING } })]);
  });

  it("reads the home again after a failed request and follows a daemon that moved", async () => {
    const first = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" });
    const second = JSON.stringify({ jsonrpc: "2.0", id: 2, method: "ping" });
    const pong = JSON.stringify({ jsonrpc: "2.0", id: 2, result: {} });
    const restarted = async (machine: Machine): Promise<Response> => {
      machine.listeners.delete(51_234);
      machine.listeners.set(52_345, process.pid);
      await recordDaemon(machine.home, { pid: process.pid, port: 52_345, startedAt: 1 });
      throw new TypeError("fetch failed", { cause: Object.assign(new Error("connect ECONNREFUSED"), { code: "ECONNREFUSED" }) });
    };
    const run = await runBridge([first, second], [restarted, new Response(pong, { status: 200 })], {
      daemon: { pid: process.pid, port: 51_234 },
    });
    expect(run.endpoints).toEqual(["http://127.0.0.1:51234/mcp", "http://127.0.0.1:52345/mcp"]);
    expect(run.stdout).toEqual([JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message: "fetch failed" } }), pong]);
  });

  it("names its caller on every relay by the URI-encoded real path it started in, without touching the body", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-bridge-caller-")));
    try {
      const checkout = join(root, "dépôt 項目");
      await mkdir(checkout);
      const link = join(root, "link");
      await symlink(checkout, link);
      const headers: string[] = [];
      const bodies: string[] = [];
      const request = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" });
      const home = join(root, "home");
      await recordDaemon(home, { pid: process.pid, port: 4321 });
      await runRerailMcpBridge({
        cwd: link,
        home,
        input: Readable.from([`${request}\n`, `${request}\n`]),
        stdout: new PassThrough(),
        stderr: new PassThrough(),
        // The bridge checks the recorded daemon's health once before its first relay.
        fetch: async (endpoint, init) => {
          if (endpoint.pathname === "/healthz") return jsonResponse(200, { status: "ok", pid: process.pid });
          headers.push(new Headers(init.headers).get("x-rerail-caller-cwd") ?? "");
          bodies.push(String(init.body));
          return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result: {} }), { status: 200 });
        },
      });
      expect(headers).toEqual([encodeURIComponent(checkout), encodeURIComponent(checkout)]);
      expect(decodeURIComponent(headers[0]!)).toBe(checkout);
      expect(bodies).toEqual([request, request]);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  it("checks a recorded daemon's health once and then relays with one state file read per request", async () => {
    const requests = [1, 2, 3].map(id => JSON.stringify({ jsonrpc: "2.0", id, method: "ping" }));
    const pongs = [1, 2, 3].map(id => JSON.stringify({ jsonrpc: "2.0", id, result: {} }));
    const run = await runBridge(requests, pongs.map(pong => new Response(pong, { status: 200 })));
    expect(run.probes).toEqual(["http://127.0.0.1:4321/healthz"]);
    expect(run.requests).toEqual(requests);
    expect(run.stdout).toEqual(pongs);
  });

  it("checks health again when the state file records a daemon started at another time", async () => {
    const first = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" });
    const second = JSON.stringify({ jsonrpc: "2.0", id: 2, method: "ping" });
    const pongs = [1, 2].map(id => JSON.stringify({ jsonrpc: "2.0", id, result: {} }));
    const restartedInPlace = async (machine: Machine): Promise<Response> => {
      await recordDaemon(machine.home, { pid: process.pid, port: 4321, startedAt: 1 });
      return new Response(pongs[0], { status: 200 });
    };
    const run = await runBridge([first, second], [restartedInPlace, new Response(pongs[1], { status: 200 })]);
    expect(run.probes).toEqual(["http://127.0.0.1:4321/healthz", "http://127.0.0.1:4321/healthz"]);
    expect(run.stdout).toEqual(pongs);
  });

  // A daemon.json left behind by a SIGKILL or a reboot whose pid now belongs to another process.
  it.each([
    ["nothing listens on its port", null],
    ["a different daemon answers on its port", process.ppid],
  ] as const)("refuses a stale state file whose pid is alive when %s, and relays nothing", async (_case, listener) => {
    const write = JSON.stringify({
      jsonrpc: "2.0",
      id: 5,
      method: "tools/call",
      params: { name: "library_rules_upsert", arguments: { id: "stale-home-rule" } },
    });
    const elsewhere = JSON.stringify({ jsonrpc: "2.0", id: 5, result: { content: [] } });
    const run = await runBridge([write], [new Response(elsewhere, { status: 200 })], { listener });
    expect(run.requests).toEqual([]);
    expect(run.stdout).toEqual([JSON.stringify({ jsonrpc: "2.0", id: 5, error: { code: -32000, message: NOT_RUNNING } })]);
    expect(run.stderr).toEqual([`rerail: mcp bridge: ${NOT_RUNNING}`]);
  });

  it("reports a daemon that stopped answering after its health check as not running", async () => {
    const lines = [1, 2, 3].map(id => JSON.stringify({ jsonrpc: "2.0", id, method: "ping" }));
    const pong = JSON.stringify({ jsonrpc: "2.0", id: 1, result: {} });
    const killed = async (machine: Machine): Promise<Response> => {
      machine.listeners.clear();
      throw new TypeError("fetch failed");
    };
    const run = await runBridge(lines, [new Response(pong, { status: 200 }), killed]);
    expect(run.requests).toEqual(lines.slice(0, 2));
    expect(run.stdout).toEqual([
      pong,
      JSON.stringify({ jsonrpc: "2.0", id: 2, error: { code: -32000, message: NOT_RUNNING } }),
      JSON.stringify({ jsonrpc: "2.0", id: 3, error: { code: -32000, message: NOT_RUNNING } }),
    ]);
  });

  it("sends the capability daemon.json records, and says a refusal is one rather than relaying it or calling the daemon down", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-bridge-access-"));
    directories.push(root);
    const home = join(root, "home");
    await recordDaemon(home, { pid: process.pid, port: 4321 });
    const state = JSON.parse(await readFile(join(home, DAEMON_STATE_FILE), "utf8")) as Record<string, unknown>;
    const request = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" });
    const pong = JSON.stringify({ jsonrpc: "2.0", id: 1, result: {} });
    const refused = JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code: -32000, message: REFUSED } });
    /** A daemon that admits only `accepted`, answering `/mcp` with `answer` once admitted. */
    const bridge = async (recorded: string, accepted: string, answer: () => Response = () => new Response(pong, { status: 200 })) => {
      await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify({ ...state, accessToken: recorded, startedAt: Math.random() }));
      const sent: string[] = [];
      const stdout = new PassThrough();
      const lines = collect(stdout);
      await runRerailMcpBridge({
        home, stdout, stderr: new PassThrough(), input: Readable.from([`${request}\n`]),
        fetch: async (endpoint, init) => {
          const access = new Headers(init.headers).get("x-rerail-access") ?? "";
          sent.push(`${endpoint.pathname} ${access}`);
          if (access !== accepted) return jsonResponse(401, { error: "access-required", message: "…" });
          return endpoint.pathname === "/healthz" ? jsonResponse(200, { status: "ok", pid: process.pid }) : answer();
        },
      });
      return { sent, stdout: lines() };
    };
    expect(await bridge("kept", "kept")).toEqual({ sent: ["/healthz kept", "/mcp kept"], stdout: [pong] });
    // A capability daemon.json no longer matches is refused at the health check: said so, not "not running".
    expect(await bridge("stale", "kept")).toEqual({ sent: ["/healthz stale"], stdout: [refused] });
    // One that stops being accepted after the check: the 401 body is no MCP answer, so the client gets the same error.
    expect(await bridge("kept", "kept", () => jsonResponse(401, { error: "access-required", message: "…" })))
      .toEqual({ sent: ["/healthz kept", "/mcp kept"], stdout: [refused] });
  });
});
