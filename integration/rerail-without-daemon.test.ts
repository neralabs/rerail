import { execFile, spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { promisify } from "node:util";
import { afterEach, describe, expect, test } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { resolveRepositoryContext } from "../src/scoped-rule-catalog.js";
import { startTestRepository, stopTestRepository } from "../test/support/started-repository.js";

const run = promisify(execFile);
const cli = join(process.cwd(), "dist", "cli.js");
const roots: string[] = [];

afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

/** A fresh checkout and an empty Rerail home: nothing autostarts, so no daemon answers. */
async function checkout(): Promise<{ root: string; env: NodeJS.ProcessEnv }> {
  const root = await mkdtemp(join(tmpdir(), "rerail-no-daemon-"));
  roots.push(root);
  await run("git", ["init", "-q"], { cwd: root });
  return { root, env: { ...process.env, RERAIL_HOME: join(root, ".rerail-home"), RERAIL_AUTOSTART: "0" } };
}

async function rerailSession(root: string, env: NodeJS.ProcessEnv, args: readonly string[]): Promise<string> {
  const { stdout } = await run(process.execPath, [cli, "session", ...args], { cwd: root, env });
  return stdout.split("\n").slice(0, 2).join("\n");
}

async function claudeStatus(root: string, env: NodeJS.ProcessEnv): Promise<string> {
  const child = spawn(process.execPath, [join(process.cwd(), "dist", "claude-status.js")], { cwd: root, env, stdio: ["pipe", "pipe", "ignore"] });
  let output = "";
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => { output += chunk; });
  child.stdin.end(JSON.stringify({ session_id: "abc", cwd: root }));
  await once(child, "close");
  return output.replaceAll(/\u001b\[[0-9;]*m|\u001b\]8;;[^\u0007]*\u0007/gu, "").trim();
}

describe("Rerail with no daemon running", () => {
  test("says not on here, with /rerail-start, in a checkout never started or stopped", async () => {
    const { root, env } = await checkout();
    const notOn = `○ Rerail is not on in ${basename(root)}.\n  Start it with /rerail-start.`;
    expect(await rerailSession(root, env, ["cursor"])).toBe(notOn);
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notOn);
    expect(await claudeStatus(root, env)).toBe("Rerail ○ not on here");

    startTestRepository(root, join(env.RERAIL_HOME!, "rerail.sqlite"));
    stopTestRepository(root, join(env.RERAIL_HOME!, "rerail.sqlite"));
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notOn);
    expect(await claudeStatus(root, env)).toBe("Rerail ○ not on here");
  }, 30_000);

  test("says not running only where the checkout is started", async () => {
    const { root, env } = await checkout();
    startTestRepository(root, join(env.RERAIL_HOME!, "rerail.sqlite"));
    const notRunning = "✕ Rerail is not running.\n  Start it with `rerail daemon start` in a terminal.";
    expect(await rerailSession(root, env, ["cursor"])).toBe(notRunning);
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notRunning);
    expect(await claudeStatus(root, env)).toBe("Rerail ✕ not running");
  }, 30_000);
});

describe("Rerail's frequent readers", () => {
  test("never write the database", async () => {
    const { root, env } = await checkout();
    const databasePath = join(env.RERAIL_HOME!, "rerail.sqlite");
    resolveRepositoryContext(root);
    openRerailDatabase(databasePath).$client.close();
    const before = await readFile(databasePath);

    const notOn = `○ Rerail is not on in ${basename(root)}.\n  Start it with /rerail-start.`;
    expect(await claudeStatus(root, env)).toBe("Rerail ○ not on here");
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notOn);
    expect(await rerailSession(root, env, ["cursor"])).toBe(notOn);
    expect(await readFile(databasePath)).toEqual(before);
  }, 30_000);

  test("never write the database, and say not running where only a writable open could tell", async () => {
    const { root, env } = await checkout();
    const databasePath = join(env.RERAIL_HOME!, "rerail.sqlite");
    resolveRepositoryContext(root);
    // A database another process is still creating has no schema yet; only a writable open waits for it.
    await mkdir(env.RERAIL_HOME!, { recursive: true });
    await writeFile(databasePath, "");

    const notRunning = "✕ Rerail is not running.\n  Start it with `rerail daemon start` in a terminal.";
    expect(await claudeStatus(root, env)).toBe("Rerail ✕ not running");
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notRunning);
    expect(await rerailSession(root, env, ["cursor"])).toBe(notRunning);
    expect((await stat(databasePath)).size).toBe(0);
  }, 30_000);
});
