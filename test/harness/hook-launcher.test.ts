import { spawnSync } from "node:child_process";
import { chmod, copyFile, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { removeFolder } from "../support/platform/remove-folder.js";

const LAUNCHER = resolve(import.meta.dirname, "../../src/harness/hook-launcher.ts");
// The launcher runs only on Windows, so its tests run there too, where a `#!/bin/sh` wrapper cannot stand in for a
// node: the recorded node is a copy of this one, told apart by its file name. Elsewhere it is a wrapper that sets
// RECORDED, since a copy of node need not start outside its install there (Homebrew's loads `@rpath/libnode`).
const WINDOWS = process.platform === "win32";
const RECORDED_NODE = WINDOWS ? "recorded-node.exe" : "recorded-node";
const RAN_RECORDED = WINDOWS ? RECORDED_NODE : "recorded";
const RAN_PATH_NODE = basename(process.execPath);

// Reports which node ran it, its argv[1], and the event on stdin, then exits non-zero like a failing hook.
const REPORTING_HOOK = [
  'import { readFileSync } from "node:fs";',
  'import { basename } from "node:path";',
  "process.stdout.write(JSON.stringify({ node: process.env.RECORDED ?? basename(process.execPath), argv1: process.argv[1], stdin: readFileSync(0, \"utf8\") }));",
  "process.exitCode = 3;",
].join("\n");

// Fails while loading, before it could answer: a missing module after an update, or a binding built for another node.
const CRASHING_HOOK = 'throw new Error("Cannot find module rerail-binding");';

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

type Recorded = "runs" | "missing" | "cannot-run";

async function launch(recorded: Recorded, hookSource = REPORTING_HOOK): Promise<{ status: number | null; stdout: string; stderr: string }> {
  const base = await mkdtemp(join(tmpdir(), "rerail-hook-launcher-"));
  temporaryDirectories.push(base);
  const hook = join(base, "claude-hook.mjs");
  await writeFile(hook, hookSource);
  // A distinct "recorded node", so the test can tell it from the node that starts the launcher.
  const node = join(base, RECORDED_NODE);
  if (recorded === "runs") await recordedNode(node);
  // A file that is no program: without the execute bit, or on Windows (which has none) not an executable at all.
  if (recorded === "cannot-run") await (WINDOWS ? writeFile(node, "not a program\n") : writeFile(node, "#!/bin/sh\n", { mode: 0o644 }));
  const result = spawnSync(process.execPath, ["--import", "tsx", LAUNCHER, node, hook], { input: '{"hook_event_name":"Stop"}', encoding: "utf8" });
  return { status: result.status, stdout: result.stdout, stderr: result.stderr };
}

async function recordedNode(path: string): Promise<void> {
  if (WINDOWS) {
    await copyFile(process.execPath, path);
    return;
  }
  await writeFile(path, `#!/bin/sh\nRECORDED=recorded exec "${process.execPath}" "$@"\n`);
  await chmod(path, 0o755);
}

describe("hook launcher started by the recorded node", () => {
  it("runs the hook in its own process, so a hook event starts one node", async () => {
    const base = await mkdtemp(join(tmpdir(), "rerail-hook-launcher-"));
    temporaryDirectories.push(base);
    const hook = join(base, "claude-hook.mjs");
    await writeFile(hook, "process.stdout.write(JSON.stringify({ pid: process.pid, argv1: process.argv[1] }));");
    // Set-up writes the recorded node as the command where `node` on PATH is another one.
    const result = spawnSync(process.execPath, ["--import", "tsx", LAUNCHER, process.execPath, hook], { input: "{}", encoding: "utf8" });
    expect(JSON.parse(result.stdout), result.stderr).toEqual({ pid: result.pid, argv1: hook });
    expect(result.status).toBe(0);
  }, 30_000);
});

describe("hook launcher", () => {
  it("runs the hook under the recorded node while that binary exists, passing its answer through", async () => {
    const { status, stdout, stderr } = await launch("runs");
    expect(JSON.parse(stdout), stderr).toMatchObject({ node: RAN_RECORDED, stdin: '{"hook_event_name":"Stop"}' });
    // A failed hook would show as an error under every tool call, so the launcher never fails.
    expect(status).toBe(0);
    expect(stderr).toContain("rerail hook: exited 3");
  });

  it("falls back to the node that started it once the recorded node is gone", async () => {
    const { status, stdout, stderr } = await launch("missing");
    // The hook serves only when it is argv[1], as if `node <hook>` had started it.
    expect(JSON.parse(stdout), stderr).toEqual({ node: RAN_PATH_NODE, argv1: expect.stringMatching(/claude-hook\.mjs$/u), stdin: '{"hook_event_name":"Stop"}' });
    expect(status).toBe(0);
  });

  it.each<Recorded>(["runs", "missing"])("answers {} and exits 0 for a hook that fails while loading (recorded node %s)", async recorded => {
    const { status, stdout, stderr } = await launch(recorded, CRASHING_HOOK);
    expect(stdout).toBe("{}\n");
    expect(status).toBe(0);
    expect(stderr).toContain("rerail hook:");
  });

  it("answers {} and exits 0 when the recorded node cannot be started", async () => {
    const { status, stdout, stderr } = await launch("cannot-run");
    expect(stdout).toBe("{}\n");
    expect(status).toBe(0);
    // The failure names the program it could not start. Its code is libuv's: EACCES without the execute bit; on Windows,
    // a file it cannot load as a program, whose code Node reports as UNKNOWN.
    expect(stderr).toMatch(WINDOWS ? /rerail hook: spawnSync .*recorded-node\.exe \w+/u : /rerail hook: .*EACCES/u);
  });
});
