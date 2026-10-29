import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, expect, it, vi } from "vitest";
import { openRerailDatabase, recordCheckout } from "../src/engine/index.js";
import { DAEMON_STATE_FILE, SqliteSessionStore, type DaemonState } from "../src/runtime/index.js";
import { RerailDaemon } from "../src/daemon/index.js";

let root: string | undefined;
let daemon: RerailDaemon | undefined;

afterEach(async () => {
  await daemon?.close();
  daemon = undefined;
  vi.unstubAllEnvs();
  if (root !== undefined) await rm(root, { recursive: true, force: true });
  root = undefined;
});

// The bridge finds the daemon through the Rerail home's state file, so the test runs beside a daemon that holds the default port.
it("bridges real MCP writes with split UTF-8 input and no response to notifications", async () => {
  root = await mkdtemp(join(tmpdir(), "rerail-mcp-bridge-"));
  vi.stubEnv("RERAIL_HOME", join(root, "home"));
  await mkdir(join(root, "home"), { recursive: true });
  const databasePath = join(root, "home", "rerail.sqlite");
  daemon = await RerailDaemon.start({
    version: "test",
    databasePath,
    workspaceRoot: root,
    host: "127.0.0.1",
    port: 0,
    stateFile: join(root, "home", DAEMON_STATE_FILE),
  });
  // A coding agent reaches only its own repository's sessions: the bridge's folder is the session's checkout.
  const database = openRerailDatabase(databasePath);
  const { repositoryKey } = recordCheckout(database, root);
  database.$client.close();
  const sessions = new SqliteSessionStore(databasePath);
  sessions.openSession({ host: "codex", sessionId: "unicode-session", cwd: root }, Date.now(), repositoryKey);
  sessions.close();
  const child = spawn(process.execPath, [resolve("dist/cli/cli.js"), "mcp-bridge"], {
    cwd: root,
    env: { ...process.env, RERAIL_ENGINE_PATH: join(root, "no-host-addon.node") },
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
    const response = await fetch(new URL("/api/sessions/unicode-session/work-links", daemon.url), { headers: { "x-rerail-access": daemon.state().accessToken! } });
    expect(await response.json()).toMatchObject({ workLinks: [{ sessionId: "unicode-session", kind: "git-branch", reference: "café" }] });
  } finally {
    if (child.exitCode === null) child.kill("SIGTERM");
  }
}, 30_000);

// A daemon.json left behind by a SIGKILL or a reboot, its pid since reused: nothing listens on the recorded port, or
// another home's daemon now holds it. The bridge must not hand that daemon this home's writes.
it.each([
  ["nothing listens on the recorded port", "closed"],
  ["another daemon holds the recorded port", "taken"],
] as const)("refuses a stale daemon state file when %s", async (_case, port) => {
  root = await mkdtemp(join(tmpdir(), "rerail-mcp-bridge-"));
  const home = join(root, "home");
  vi.stubEnv("RERAIL_HOME", home);
  await mkdir(join(root, "other-home"), { recursive: true });
  await mkdir(home, { recursive: true });
  const otherStateFile = join(root, "other-home", DAEMON_STATE_FILE);
  daemon = await RerailDaemon.start({
    version: "test",
    databasePath: join(root, "other-home", "rerail.sqlite"),
    workspaceRoot: root,
    host: "127.0.0.1",
    port: 0,
    stateFile: otherStateFile,
  });
  const otherState = JSON.parse(await readFile(otherStateFile, "utf8")) as DaemonState;
  // The parent pid is alive but is not the daemon's: the daemon runs in this process and answers /healthz with its pid.
  const stale: DaemonState = port === "closed"
    ? { ...otherState, pid: process.ppid, port: 1, url: "http://127.0.0.1:1/" }
    : { ...otherState, pid: process.ppid };
  await writeFile(join(home, DAEMON_STATE_FILE), JSON.stringify(stale));
  const lines = [
    { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "bridge-test", version: "1" } } },
    { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "work_links_add", arguments: { sessionId: "stale-home", kind: "git-branch", reference: "stale" } } },
  ];
  const child = spawn(process.execPath, [resolve("dist/cli/cli.js"), "mcp-bridge"], {
    cwd: root,
    env: { ...process.env, RERAIL_ENGINE_PATH: join(root, "no-host-addon.node") },
    stdio: ["pipe", "pipe", "pipe"],
  });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8").on("data", chunk => { stdout += chunk; });
  child.stderr.setEncoding("utf8").on("data", chunk => { stderr += chunk; });
  try {
    const exit = once(child, "exit");
    child.stdin.end(lines.map(line => JSON.stringify(line)).join("\n") + "\n");
    const [code] = await exit;
    expect(code, stderr).toBe(0);
    const message = "Rerail daemon is not running; start it with rerail daemon start";
    expect(stdout.trim().split("\n").map(line => JSON.parse(line) as unknown)).toEqual([1, 2].map(id => ({
      jsonrpc: "2.0",
      id,
      error: { code: -32000, message },
    })));
    const response = await fetch(new URL("/api/sessions/stale-home/work-links", daemon.url), { headers: { "x-rerail-access": daemon.state().accessToken! } });
    expect(await response.json()).toMatchObject({ workLinks: [] });
  } finally {
    if (child.exitCode === null) child.kill("SIGTERM");
  }
}, 30_000);
