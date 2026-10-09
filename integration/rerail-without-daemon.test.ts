import { execFile, spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { promisify } from "node:util";
import { afterEach, describe, expect, test } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { resolveCloneId } from "../src/repository-identity.js";
import { workspaceStartState } from "../src/repository-start.js";
import { setUpRepository } from "../src/rule-configuration.js";
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

/** Start `root`'s repository and set it up with no rules, so rules would run there. */
function startSetUp(root: string, databasePath: string): void {
  const repositoryKey = startTestRepository(root, databasePath);
  const database = openRerailDatabase(databasePath);
  try {
    setUpRepository(database, repositoryKey, { packs: [] });
  } finally {
    database.$client.close();
  }
}

describe("Rerail with no daemon running", () => {
  test("says not on here, with /rerail-start, in a checkout not yet started or stopped", async () => {
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

  test("says not set up, with /rerail-start, in a started checkout whose repository is not set up", async () => {
    const { root, env } = await checkout();
    startTestRepository(root, join(env.RERAIL_HOME!, "rerail.sqlite"));
    const notSetUp = `○ Rerail is on in ${basename(root)}, which is not set up yet: no rules are checked.\n  Set it up with /rerail-start (rerail start).`;
    expect(await rerailSession(root, env, ["cursor"])).toBe(notSetUp);
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notSetUp);
    expect(await claudeStatus(root, env)).toBe("Rerail ○ not set up  ·  rerail start");
  }, 30_000);

  test("says not running only where the checkout is started and set up", async () => {
    const { root, env } = await checkout();
    startSetUp(root, join(env.RERAIL_HOME!, "rerail.sqlite"));
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
    // The checkout has its clone identity, but was never recorded: Rerail is stopped there.
    resolveCloneId(root);
    openRerailDatabase(databasePath).$client.close();
    const before = await readFile(databasePath);
    expect(workspaceStartState(root, databasePath)).toBe("stopped");

    expect(await claudeStatus(root, env)).toBe("Rerail ○ not on here");
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(`○ Rerail is not on in ${basename(root)}.\n  Start it with /rerail-start.`);
    expect(await rerailSession(root, env, ["cursor"])).toContain("○ Rerail is not on in");
    expect(await readFile(databasePath)).toEqual(before);
  }, 30_000);
});
