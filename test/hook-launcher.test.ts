import { spawnSync } from "node:child_process";
import { chmod, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const LAUNCHER = resolve(import.meta.dirname, "../src/hook-launcher.ts");

// Reports which node ran it, its argv[1], and the event on stdin, then exits non-zero like a failing hook.
const HOOK = [
  'import { readFileSync } from "node:fs";',
  'process.stdout.write(JSON.stringify({ node: process.env.RECORDED ?? "path", argv1: process.argv[1], stdin: readFileSync(0, "utf8") }));',
  "process.exitCode = 3;",
].join("\n");

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function launch(recordedExists: boolean): Promise<{ status: number | null; report: unknown; stderr: string }> {
  const base = await mkdtemp(join(tmpdir(), "rerail-hook-launcher-"));
  temporaryDirectories.push(base);
  const hook = join(base, "claude-hook.mjs");
  await writeFile(hook, HOOK);
  // A distinct "recorded node": a wrapper that marks the run, so the test can tell it from the PATH node.
  const recorded = join(base, "recorded-node");
  if (recordedExists) {
    await writeFile(recorded, `#!/bin/sh\nRECORDED=recorded exec "${process.execPath}" "$@"\n`);
    await chmod(recorded, 0o755);
  }
  const result = spawnSync(process.execPath, ["--import", "tsx", LAUNCHER, recorded, hook], { input: '{"hook_event_name":"Stop"}', encoding: "utf8" });
  return { status: result.status, report: result.stdout === "" ? undefined : JSON.parse(result.stdout), stderr: result.stderr };
}

describe.skipIf(process.platform === "win32")("hook launcher", () => {
  it("runs the hook under the recorded node while that binary exists", async () => {
    const { status, report, stderr } = await launch(true);
    expect(report, stderr).toMatchObject({ node: "recorded", stdin: '{"hook_event_name":"Stop"}' });
    expect(status).toBe(3);
  });

  it("falls back to the node that started it once the recorded node is gone", async () => {
    const { status, report, stderr } = await launch(false);
    // The hook serves only when it is argv[1], as if `node <hook>` had started it.
    expect(report, stderr).toEqual({ node: "path", argv1: expect.stringMatching(/claude-hook\.mjs$/u), stdin: '{"hook_event_name":"Stop"}' });
    expect(status).toBe(3);
  });
});
