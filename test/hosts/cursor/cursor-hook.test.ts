import { spawnSync } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const HOOK = resolve(import.meta.dirname, "../../../src/hosts/cursor/cursor-hook.ts");

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

/** Runs the Cursor hook as Cursor does, `node <hook>` with the event on stdin, against its own empty Rerail home. */
async function runHook(stdin: string): Promise<{ status: number | null; stdout: string; stderr: string }> {
  const home = await mkdtemp(join(tmpdir(), "rerail-cursor-hook-"));
  temporaryDirectories.push(home);
  const result = spawnSync(process.execPath, ["--import", "tsx", HOOK], {
    input: stdin, encoding: "utf8", env: { ...process.env, RERAIL_HOME: home },
  });
  return { status: result.status, stdout: result.stdout, stderr: result.stderr };
}

describe("Cursor hook process", () => {
  it("answers {} and exits 0 on input it cannot read, so Cursor shows no hook error", async () => {
    const { status, stdout, stderr } = await runHook("{not json");
    expect(stdout).toBe("{}\n");
    expect(status).toBe(0);
    expect(stderr).toMatch(/^rerail hook: /u);
  });
});
