import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough, Readable } from "node:stream";
import { afterEach, describe, expect, it, vi } from "vitest";
import { runRerailMcpBridge } from "../src/dev-bridge.js";

type FetchStep = Response | Error;

interface BridgeRun {
  stdout: readonly string[];
  stderr: readonly string[];
  requests: readonly string[];
}

const directories: string[] = [];

function jsonResponse(status: number, body: unknown, contentType = "application/json"): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": contentType } });
}

function collect(stream: PassThrough): () => string[] {
  const chunks: Buffer[] = [];
  stream.on("data", (chunk) => chunks.push(Buffer.from(chunk)));
  return () => Buffer.concat(chunks).toString("utf8").split("\n").filter(Boolean);
}

async function runBridge(
  lines: readonly string[],
  steps: readonly FetchStep[],
  prepare?: (root: string) => Promise<string | undefined>,
): Promise<BridgeRun> {
  const root = await mkdtemp(join(tmpdir(), "rerail-bridge-"));
  directories.push(root);
  const cwd = await prepare?.(root) ?? root;
  const requests: string[] = [];
  const remaining = [...steps];
  const fetchImpl = vi.fn(async (_endpoint: URL, init: RequestInit): Promise<Response> => {
    requests.push(String(init.body));
    const step = remaining.shift();
    if (step === undefined) throw new Error("unexpected request");
    if (step instanceof Error) throw step;
    return step;
  });
  const stdout = new PassThrough();
  const stderr = new PassThrough();
  const collectedStdout = collect(stdout);
  const collectedStderr = collect(stderr);
  await runRerailMcpBridge({
    cwd,
    input: Readable.from(lines.map((line) => `${line}\n`)),
    stdout,
    stderr,
    fetch: fetchImpl,
  });
  return { stdout: collectedStdout(), stderr: collectedStderr(), requests };
}

afterEach(async () => {
  await Promise.all(directories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe("Rerail MCP bridge", () => {
  it("relays successful responses verbatim", async () => {
    const request = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "ping" });
    const result = JSON.stringify({ jsonrpc: "2.0", id: 1, result: {} });
    const run = await runBridge([request], [new Response(result, { status: 200 })]);
    expect(run.requests).toEqual([request]);
    expect(run.stdout).toEqual([result]);
    expect(run.stderr).toEqual([]);
  });

  it("does not connect a nested disabled workspace to the shared MCP endpoint", async () => {
    const request = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/list" });
    const run = await runBridge([request], [], async root => {
      const worktree = join(root, "tmp", "baseline-gate-audit");
      await mkdir(join(root, ".nera"));
      await mkdir(worktree, { recursive: true });
      await writeFile(join(root, ".nera", "instance.env"), 'NERA_RERAIL_ENABLED="false"\n');
      return worktree;
    });

    expect(run.requests).toEqual([]);
    expect(run.stdout).toHaveLength(1);
    expect(JSON.parse(run.stdout[0]!)).toMatchObject({
      jsonrpc: "2.0",
      id: 1,
      error: { code: -32000, message: expect.stringContaining("Rerail is disabled for workspace") },
    });
    expect(run.stderr).toEqual([expect.stringContaining("Rerail is disabled for workspace")]);
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
});
