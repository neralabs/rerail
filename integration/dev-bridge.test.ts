import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { copyFile, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, expect, it, vi } from "vitest";
import { RerailDaemon } from "../src/daemon.js";

let root: string | undefined;
let daemon: RerailDaemon | undefined;

afterEach(async () => {
  await daemon?.close();
  daemon = undefined;
  vi.unstubAllEnvs();
  if (root !== undefined) await rm(root, { recursive: true, force: true });
  root = undefined;
});

it("bridges real MCP writes with split UTF-8 input and no response to notifications", async () => {
  root = await mkdtemp(join(tmpdir(), "rerail-mcp-bridge-"));
  vi.stubEnv("RERAIL_HOME", join(root, "home"));
  daemon = await RerailDaemon.start({
    version: "test",
    databasePath: join(root, "home", "rerail.sqlite"),
    workspaceRoot: root,
    host: "127.0.0.1",
    port: 0,
  });
  await mkdir(join(root, ".nera"));
  await writeFile(join(root, ".nera", "instance.env"), `NERA_RERAIL_ENABLED="true"\nNERA_RERAIL_PUBLIC_URL="${daemon.url}"\n`);
  const child = spawn(process.execPath, [resolve("dist/cli.js"), "mcp-bridge"], {
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

it("rejects arbitrary build destinations without deleting their contents", async () => {
  root = await mkdtemp(join(tmpdir(), "rerail-bridge-output-"));
  const sentinel = join(root, "keep.txt");
  const content = "User-owned build notes\n";
  await writeFile(sentinel, content);
  const result = spawnSync(process.execPath, [resolve("scripts/build-dev-bridge.mjs"), root], { encoding: "utf8" });
  expect(result.status, result.stderr).toBe(1);
  expect(await readFile(sentinel, "utf8")).toBe(content);
});

it("rejects a symlinked build root before deleting an outside artifact", async () => {
  root = await mkdtemp(join(tmpdir(), "rerail-bridge-symlink-"));
  const fixture = join(root, "rerail");
  const builder = join(fixture, "scripts/build-dev-bridge.mjs");
  await mkdir(dirname(builder), { recursive: true });
  await copyFile(resolve("scripts/build-dev-bridge.mjs"), builder);
  await symlink(resolve("node_modules"), join(fixture, "node_modules"), "junction");
  const outside = join(root, "outside");
  await mkdir(join(outside, "host-bridge"), { recursive: true });
  const sentinel = join(outside, "host-bridge", "keep.txt");
  const content = "Outside artifact\n";
  await writeFile(sentinel, content);
  await symlink(outside, join(fixture, "dist"), "junction");
  const result = spawnSync(process.execPath, [builder], { encoding: "utf8" });
  expect(result.status, result.stderr).toBe(1);
  expect(await readFile(sentinel, "utf8")).toBe(content);
});
