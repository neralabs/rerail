import { spawnSync } from "node:child_process";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const LAUNCHER = resolve(import.meta.dirname, "../../src/harness/hook-launcher.ts");

// Reports which node ran it, its argv[1], and the event on stdin, then exits non-zero like a failing hook.
const REPORTING_HOOK = [
  'import { readFileSync } from "node:fs";',
  'process.stdout.write(JSON.stringify({ node: process.env.RECORDED ?? "path", argv1: process.argv[1], stdin: readFileSync(0, "utf8") }));',
  "process.exitCode = 3;",
].join("\n");

// Fails while loading, before it could answer: a missing module after an update, or a binding built for another node.
const CRASHING_HOOK = 'throw new Error("Cannot find module rerail-binding");';

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

type Recorded = "runs" | "missing" | "cannot-run";

async function launch(recorded: Recorded, hookSource = REPORTING_HOOK): Promise<{ status: number | null; stdout: string; stderr: string }> {
  const base = await mkdtemp(join(tmpdir(), "rerail-hook-launcher-"));
  temporaryDirectories.push(base);
  const hook = join(base, "claude-hook.mjs");
  await writeFile(hook, hookSource);
  // A distinct "recorded node": a wrapper that marks the run, so the test can tell it from the PATH node.
  const node = join(base, "recorded-node");
  if (recorded !== "missing") {
    await writeFile(node, `#!/bin/sh\nRECORDED=recorded exec "${process.execPath}" "$@"\n`);
    await chmod(node, recorded === "runs" ? 0o755 : 0o644);
  }
  const result = spawnSync(process.execPath, ["--import", "tsx", LAUNCHER, node, hook], { input: '{"hook_event_name":"Stop"}', encoding: "utf8" });
  return { status: result.status, stdout: result.stdout, stderr: result.stderr };
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

  it.each([
    ["ends the process itself", 'process.stdout.write("{}"); process.exit(3);'],
    ["throws once it has loaded", 'setTimeout(() => { throw new Error("late failure"); }, 0);'],
  ])("exits 0 for a hook that %s, as it does for a hook it runs as a child", async (_case, source) => {
    const base = await mkdtemp(join(tmpdir(), "rerail-hook-launcher-"));
    temporaryDirectories.push(base);
    const hook = join(base, "claude-hook.mjs");
    await writeFile(hook, source);
    const result = spawnSync(process.execPath, ["--import", "tsx", LAUNCHER, process.execPath, hook], { input: "{}", encoding: "utf8" });
    expect(result.status, result.stderr).toBe(0);
    expect(result.stderr).toContain("rerail hook: exited");
  }, 30_000);
});

describe.skipIf(process.platform === "win32")("hook launcher", () => {
  it("runs the hook under the recorded node while that binary exists, passing its answer through", async () => {
    const { status, stdout, stderr } = await launch("runs");
    expect(JSON.parse(stdout), stderr).toMatchObject({ node: "recorded", stdin: '{"hook_event_name":"Stop"}' });
    // A failed hook would show as an error under every tool call, so the launcher never fails.
    expect(status).toBe(0);
    expect(stderr).toContain("rerail hook: exited 3");
  });

  it("falls back to the node that started it once the recorded node is gone", async () => {
    const { status, stdout, stderr } = await launch("missing");
    // The hook serves only when it is argv[1], as if `node <hook>` had started it.
    expect(JSON.parse(stdout), stderr).toEqual({ node: "path", argv1: expect.stringMatching(/claude-hook\.mjs$/u), stdin: '{"hook_event_name":"Stop"}' });
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
    expect(stderr).toMatch(/rerail hook: .*EACCES/u);
  });
});
