import { execFile, spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import type { AddressInfo } from "node:net";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { promisify } from "node:util";
import { afterEach, describe, expect, test } from "vitest";
import { openRerailDatabase, resolveCloneId, setUpRepository } from "../src/engine/index.js";
import { workspaceStartState } from "../src/runtime/index.js";
import { holdEngineEndpoint, testEngineEndpoint } from "../test/support/platform/engine-endpoint.js";
import { startTestRepository, stopTestRepository } from "../test/support/started-repository.js";

const run = promisify(execFile);
const cli = join(process.cwd(), "dist", "cli", "cli.js");
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
  const child = spawn(process.execPath, [join(process.cwd(), "dist", "hosts", "claude", "claude-status.js")], { cwd: root, env, stdio: ["pipe", "pipe", "ignore"] });
  let output = "";
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => { output += chunk; });
  child.stdin.end(JSON.stringify({ session_id: "abc", cwd: root }));
  await once(child, "close");
  return output.replaceAll(/\u001b\[[0-9;]*m|\u001b\]8;;[^\u0007]*\u0007/gu, "").trim();
}

/** `rerail session`'s first two lines and its exit code, for a run that may end with a code other than 0. */
async function rerailSessionExit(root: string, env: NodeJS.ProcessEnv, args: readonly string[]): Promise<{ lines: string; code: number | null }> {
  const child = spawn(process.execPath, [cli, "session", ...args], { cwd: root, env, stdio: ["ignore", "pipe", "ignore"] });
  let output = "";
  child.stdout.setEncoding("utf8").on("data", (chunk: string) => { output += chunk; });
  const [code] = await once(child, "close") as [number | null];
  return { lines: output.split("\n").slice(0, 2).join("\n"), code };
}

/**
 * A listener recorded in `home`'s daemon.json under this process's live pid, as a running daemon is. It takes every
 * request and never answers it, as a daemon whose main thread is busy does, except the paths `answers` gives a status.
 */
async function listenAsDaemon(home: string, answers: Readonly<Record<string, number>> = {}): Promise<{ asked: string[]; close: () => Promise<void> }> {
  const asked: string[] = [];
  // On Windows a recorded daemon counts as alive only while its engine pipe exists (`recordedDaemonAlive`).
  const kernelSocketPath = testEngineEndpoint(home);
  const releaseEndpoint = await holdEngineEndpoint(kernelSocketPath);
  const daemon = createServer((request, response) => {
    const path = new URL(request.url ?? "/", "http://daemon").pathname;
    asked.push(path);
    if (Object.hasOwn(answers, path)) response.writeHead(answers[path]!).end();
  });
  await new Promise<void>((resolve, reject) => daemon.once("error", reject).listen(0, "127.0.0.1", () => resolve()))
    .catch(async (error: unknown) => { await releaseEndpoint(); throw error; });
  const close = async (): Promise<void> => {
    daemon.closeAllConnections();
    await new Promise<void>(resolve => daemon.close(() => resolve()));
    await releaseEndpoint();
  };
  const { port } = daemon.address() as AddressInfo;
  await writeFile(join(home, "daemon.json"), JSON.stringify({
    pid: process.pid, host: "127.0.0.1", port, url: `http://127.0.0.1:${port}/`, version: "test", ui: "bundled",
    databasePath: join(home, "rerail.sqlite"), kernelSocketPath, startedAt: 1, packageRoot: process.cwd(),
  })).catch(async (error: unknown) => { await close(); throw error; });
  return { asked, close };
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
  test("says not on here, with /rerail-start, and draws no footer, in a checkout not yet started or stopped", async () => {
    const { root, env } = await checkout();
    const notOn = `○ Rerail is not on in ${basename(root)}.\n  Start it with /rerail-start.`;
    expect(await rerailSession(root, env, ["cursor"])).toBe(notOn);
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notOn);
    expect(await claudeStatus(root, env)).toBe("");

    startTestRepository(root, join(env.RERAIL_HOME!, "rerail.sqlite"));
    stopTestRepository(root, join(env.RERAIL_HOME!, "rerail.sqlite"));
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notOn);
    expect(await claudeStatus(root, env)).toBe("");
  }, 30_000);

  test("says not set up, with /rerail-start, in a started checkout whose repository is not set up", async () => {
    const { root, env } = await checkout();
    startTestRepository(root, join(env.RERAIL_HOME!, "rerail.sqlite"));
    const notSetUp = `○ Rerail is on in ${basename(root)}, which is not set up yet: no rules are checked.\n  Set it up with /rerail-start (rerail start).`;
    expect(await rerailSession(root, env, ["cursor"])).toBe(notSetUp);
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notSetUp);
    expect(await claudeStatus(root, env)).toBe("R E R A I L ○ not set up · rerail start");
  }, 30_000);

  test("says not running only where the checkout is started and set up", async () => {
    const { root, env } = await checkout();
    startSetUp(root, join(env.RERAIL_HOME!, "rerail.sqlite"));
    const notRunning = "✕ Rerail is not running.\n  Start it with `rerail daemon start` in a terminal.";
    expect(await rerailSession(root, env, ["cursor"])).toBe(notRunning);
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notRunning);
    expect(await claudeStatus(root, env)).toBe("R E R A I L ✕ not running");
  }, 30_000);
});

describe("Rerail with a daemon that does not answer", () => {
  const notAnswering = "● Rerail is running, but its daemon did not answer in time.\n"
    + "  Ask again in a moment; if it keeps happening, run `rerail status` in a terminal.";

  test("the status line and rerail session say the daemon is not answering, not how to start it", async () => {
    const { root, env } = await checkout();
    const home = env.RERAIL_HOME!;
    startSetUp(root, join(home, "rerail.sqlite"));
    const daemon = await listenAsDaemon(home);
    try {
      expect(await claudeStatus(root, env)).toBe("R E R A I L ● not answering");
      expect(await rerailSession(root, env, ["cursor"])).toBe(notAnswering);
      expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notAnswering);
      // The status line reads once; rerail session then asks for the dashboard link, which `--open` needs.
      expect(daemon.asked).toEqual(["/api/presentation", "/api/presentation", "/healthz", "/api/presentation", "/healthz"]);
    } finally {
      await daemon.close();
    }
  }, 30_000);

  test("rerail session --open asks the daemon again for the dashboard link after a read that did not answer", async () => {
    const { root, env } = await checkout();
    const home = env.RERAIL_HOME!;
    startSetUp(root, join(home, "rerail.sqlite"));
    // The daemon answers the second read, with an error here so no browser opens: the page it would give opens otherwise.
    const daemon = await listenAsDaemon(home, { "/healthz": 503 });
    try {
      expect(await rerailSessionExit(root, env, ["cursor", "--open"])).toEqual({ lines: notAnswering, code: 3 });
      expect(await rerailSessionExit(root, env, ["claude-code", "abc", "--open"])).toEqual({ lines: notAnswering, code: 3 });
      expect(daemon.asked).toEqual(["/api/presentation", "/healthz", "/api/presentation", "/healthz"]);
    } finally {
      await daemon.close();
    }
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

    expect(await claudeStatus(root, env)).toBe("");
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(`○ Rerail is not on in ${basename(root)}.\n  Start it with /rerail-start.`);
    expect(await rerailSession(root, env, ["cursor"])).toContain("○ Rerail is not on in");
    expect(await readFile(databasePath)).toEqual(before);
  }, 30_000);

  test("never write the database, and say not running where only a writable open could tell", async () => {
    const { root, env } = await checkout();
    const databasePath = join(env.RERAIL_HOME!, "rerail.sqlite");
    resolveCloneId(root);
    // A database another process is still creating has no schema yet; only a writable open waits for it.
    await mkdir(env.RERAIL_HOME!, { recursive: true });
    await writeFile(databasePath, "");

    const notRunning = "✕ Rerail is not running.\n  Start it with `rerail daemon start` in a terminal.";
    expect(await claudeStatus(root, env)).toBe("R E R A I L ✕ not running");
    expect(await rerailSession(root, env, ["claude-code", "abc"])).toBe(notRunning);
    expect(await rerailSession(root, env, ["cursor"])).toBe(notRunning);
    expect((await stat(databasePath)).size).toBe(0);
  }, 30_000);
});
