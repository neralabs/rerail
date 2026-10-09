import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, expect, it, vi } from "vitest";
import { DEFAULT_DAEMON_PORT, RerailDaemon } from "../src/daemon/index.js";

/** The bridge always dials the default daemon port; a daemon already running on this machine owns it. */
async function defaultPortTaken(): Promise<boolean> {
  const probe = createServer();
  try {
    probe.listen(DEFAULT_DAEMON_PORT, "127.0.0.1");
    await once(probe, "listening");
    return false;
  } catch {
    return true;
  } finally {
    probe.close();
  }
}

let root: string | undefined;
let daemon: RerailDaemon | undefined;

afterEach(async () => {
  await daemon?.close();
  daemon = undefined;
  vi.unstubAllEnvs();
  if (root !== undefined) await rm(root, { recursive: true, force: true });
  root = undefined;
});

it.skipIf(await defaultPortTaken())("bridges real MCP writes with split UTF-8 input and no response to notifications", async () => {
  root = await mkdtemp(join(tmpdir(), "rerail-mcp-bridge-"));
  vi.stubEnv("RERAIL_HOME", join(root, "home"));
  daemon = await RerailDaemon.start({
    version: "test",
    databasePath: join(root, "home", "rerail.sqlite"),
    workspaceRoot: root,
    host: "127.0.0.1",
    port: DEFAULT_DAEMON_PORT,
  });
  const child = spawn(process.execPath, [resolve("dist/cli/cli.js"), "mcp-bridge"], {
    cwd: root,
    env: { ...process.env, RERAIL_NATIVE_KERNEL_PATH: join(root, "no-host-addon.node") },
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", chunk => { stdout += chunk; });
  child.stderr.setEncoding("utf8").on("data", chunk => { stderr += chunk; });
  try {
    const lines = [
      { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "bridge-test", version: "1" } } },
      { jsonrpc: "2.0", method: "notifications/initialized" },
      { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "work_links_add", arguments: { sessionId: "unicode-session", kind: "git-branch", reference: "café" } } },
    ];
    const input = Buffer.from(lines.map(line => JSON.stringify(line)).join("\n") + "\n");
    const split = input.indexOf(Buffer.from("é")) + 1;
    const firstResponse = once(child.stdout, "data");
    const exit = once(child, "exit");
    child.stdin.write(input.subarray(0, split));
    await Promise.race([
      firstResponse,
      exit.then(() => { throw new Error(`Bridge exited before initialization: ${stderr}`); }),
    ]);
    child.stdin.end(input.subarray(split));
    const [code] = await exit;
    expect(code, stderr).toBe(0);
    const responses = stdout.trim().split("\n").map(line => JSON.parse(line) as { id: number; error?: unknown });
    expect(responses.map(response => response.id)).toEqual([1, 2]);
    expect(responses.every(response => response.error === undefined)).toBe(true);
    const response = await fetch(new URL("/api/sessions/unicode-session/work-links", daemon.url));
    expect(await response.json()).toMatchObject({ workLinks: [{ sessionId: "unicode-session", kind: "git-branch", reference: "café" }] });
  } finally {
    if (child.exitCode === null) child.kill("SIGTERM");
  }
}, 30_000);
