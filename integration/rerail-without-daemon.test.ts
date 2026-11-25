import { execFile, spawn } from "node:child_process";
import { once } from "node:events";
import { appendFile, cp, mkdir, mkdtemp, readFile, realpath, rm, stat, writeFile } from "node:fs/promises";
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

/** What the listener answers on a path: a status with no body, or 200 with the JSON a request's URL gets. */
type Answer = number | ((url: URL) => unknown);

/**
 * A listener recorded in `home`'s daemon.json under this process's live pid, as a running daemon is. It takes every
 * request and never answers it, as a daemon whose main thread is busy does, except the paths `answers` names.
 */
async function listenAsDaemon(home: string, answers: Readonly<Record<string, Answer>> = {}): Promise<{ asked: string[]; port: number; close: () => Promise<void> }> {
  const asked: string[] = [];
  // On Windows a recorded daemon counts as alive only while its engine pipe exists (`recordedDaemonAlive`).
  const kernelSocketPath = testEngineEndpoint(home);
  const releaseEndpoint = await holdEngineEndpoint(kernelSocketPath);
  const daemon = createServer((request, response) => {
    const url = new URL(request.url ?? "/", "http://daemon");
    asked.push(url.pathname);
    const answer = Object.hasOwn(answers, url.pathname) ? answers[url.pathname] : undefined;
    if (typeof answer === "number") response.writeHead(answer).end();
    else if (answer !== undefined) response.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(answer(url)));
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
  return { asked, port, close };
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
      // Each reads once: these lines show no link, and without --open no page is wanted.
      expect(daemon.asked).toEqual(["/api/presentation", "/api/presentation", "/api/presentation"]);
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

describe("Rerail with a daemon that answers", () => {
  /**
   * A started, set-up checkout and a listener that answers as its daemon: the presentation read with the repository and
   * checkout it is in, a session `abc` with one rule waiting for approval, and the health and repository reads too.
   */
  async function answeringDaemon(): Promise<{ root: string; env: NodeJS.ProcessEnv; rulesPage: string; daemon: Awaited<ReturnType<typeof listenAsDaemon>> }> {
    const { root, env } = await checkout();
    const home = env.RERAIL_HOME!;
    startSetUp(root, join(home, "rerail.sqlite"));
    const checkoutPath = await realpath(root);
    const scope = { repositoryKey: "remote:example.com/acme/web", checkoutPath };
    const facts = { pid: process.pid, database: { path: join(home, "rerail.sqlite"), exists: true }, native: { ok: true } };
    const session = { session: { sessionId: "abc", host: "claude-code", cwd: checkoutPath }, hud: { activeRuleCount: 2, steeringCount: 0, awaitingApproval: 1 }, audit: null };
    const daemon = await listenAsDaemon(home, {
      "/api/presentation": url => ({
        presence: "compact", repositoryState: "started", scope,
        ...(url.searchParams.has("sessionId") ? { context: { daemon: facts, session } } : {}),
      }),
      "/healthz": () => ({ status: "ok", ...facts }),
      "/api/repository": () => ({ repository: { repositoryKey: scope.repositoryKey }, checkout: { checkoutPath } }),
    });
    const page = new URL(`http://127.0.0.1:${daemon.port}/`);
    page.searchParams.set("repository", scope.repositoryKey);
    page.searchParams.set("checkout", checkoutPath);
    page.hash = "rules";
    return { root, env: { ...env, BROWSER: "none" }, rulesPage: page.href, daemon };
  }

  test("rerail session links the rules page from the daemon's one answer, with and without a session", async () => {
    const { root, env, rulesPage, daemon } = await answeringDaemon();
    try {
      const { stdout: cursor } = await run(process.execPath, [cli, "session", "cursor"], { cwd: root, env });
      expect(cursor).toContain(`  Manage rules → ${rulesPage}\n`);
      const { stdout: claude } = await run(process.execPath, [cli, "session", "claude-code", "abc"], { cwd: root, env });
      expect(claude).toContain(`  1 to approve, running in Log → ${rulesPage}\n`);
      expect(daemon.asked).toEqual(["/api/presentation", "/api/presentation"]);
    } finally {
      await daemon.close();
    }
  }, 30_000);

  test("rerail session --open opens the rules page the daemon's one answer gives", async () => {
    const { root, env, rulesPage, daemon } = await answeringDaemon();
    try {
      const { stdout } = await run(process.execPath, [cli, "session", "cursor", "--open"], { cwd: root, env });
      expect(stdout).toContain(`BROWSER is set to none, so no browser opened. Open ${rulesPage}\n`);
      expect(daemon.asked).toEqual(["/api/presentation"]);
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

/** A checkout with `top.txt` changed and a checked-out submodule `sub` with a changed file, its Git folder's config whole or empty. */
async function checkoutWithSubmodule(root: string, config: "whole" | "empty"): Promise<string> {
  const git = (cwd: string, ...args: string[]): Promise<unknown> =>
    run("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "protocol.file.allow=always", ...args], { cwd });
  const source = join(root, `source-${config}`);
  const work = join(root, `work-${config}`);
  for (const [repository, file] of [[source, "inner.txt"], [work, "top.txt"]] as const) {
    await mkdir(repository);
    await git(repository, "init", "-q");
    await writeFile(join(repository, file), "one\n");
    await git(repository, "add", ".");
    await git(repository, "commit", "-q", "-m", "seed");
  }
  await git(work, "submodule", "add", "-q", source, "sub");
  await git(work, "commit", "-q", "-m", "submodule");
  if (config === "empty") await writeFile(join(work, ".git", "modules", "sub", "config"), "");
  await writeFile(join(work, "top.txt"), "two\n");
  await writeFile(join(work, "sub", "inner.txt"), "two\n");
  return work;
}

/**
 * A checkout with `top.txt` changed and `count` checked-out submodules, each a copy of one repository of ten files, the
 * first with a changed file.
 */
async function checkoutWithSubmodules(root: string, count: number): Promise<string> {
  const git = async (cwd: string, ...args: string[]): Promise<string> =>
    (await run("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd })).stdout;
  const source = join(root, "source");
  const work = join(root, "work");
  for (const [repository, files] of [[source, 10], [work, 1]] as const) {
    await mkdir(repository);
    await git(repository, "init", "-q");
    for (let file = 0; file < files; file += 1) await writeFile(join(repository, `f${file}.txt`), "one\n");
    await git(repository, "add", ".");
    await git(repository, "commit", "-q", "-m", "seed");
  }
  const commit = (await git(source, "rev-parse", "HEAD")).trim();
  const names = Array.from({ length: count }, (_, at) => `sub${String(at).padStart(3, "0")}`);
  for (const name of names) {
    await cp(source, join(work, name), { recursive: true });
    await git(join(work, name), "update-index", "-q", "--refresh");
    await appendFile(join(work, ".gitmodules"), `[submodule "${name}"]\n\tpath = ${name}\n\turl = ../source\n`);
  }
  await git(work, "update-index", "--add", ...names.flatMap(name => ["--cacheinfo", `160000,${commit},${name}`]));
  await git(work, "add", ".gitmodules");
  await git(work, "commit", "-q", "-m", "submodules");
  await writeFile(join(work, "f0.txt"), "two\n");
  await writeFile(join(work, "sub000", "f0.txt"), "two\n");
  return work;
}

/**
 * `rerail scan --scope git-diff --json` in `cwd` with no daemon: its exit code, the paths its report lists, and what it
 * printed to stderr where it exited with another code than 0, so a scan that failed says why in the case's failure.
 */
async function scannedPaths(cwd: string, env: NodeJS.ProcessEnv): Promise<[number, unknown, string]> {
  const ended = await run(process.execPath, [cli, "scan", "--scope", "git-diff", "--json"], { cwd, env }).then(
    ({ stdout }) => ({ code: 0, stdout, stderr: "" }),
    (error: { code?: number; stdout?: string; stderr?: string }) => ({ code: error.code ?? -1, stdout: error.stdout ?? "", stderr: error.stderr ?? "" }),
  );
  const report = ((): { paths?: unknown } => {
    try {
      return JSON.parse(ended.stdout) as { paths?: unknown };
    } catch {
      return {};
    }
  })();
  return [ended.code, report.paths, ended.stderr];
}

describe("rerail scan with no daemon", () => {
  test("lists a changed submodule whose Git folder's config holds no entry as it lists one whose config is whole", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-no-daemon-"));
    roots.push(root);
    const env = { ...process.env, RERAIL_HOME: join(root, ".rerail-home"), RERAIL_AUTOSTART: "0", RERAIL_EXPERIMENTAL_HEADLESS_SCAN: "1" };

    const scans = [await scannedPaths(await checkoutWithSubmodule(root, "whole"), env), await scannedPaths(await checkoutWithSubmodule(root, "empty"), env)];

    // Each scan against the answer, so a failure names the checkout whose scan failed and what that scan said.
    expect(scans).toEqual([[0, ["sub", "top.txt"], ""], [0, ["sub", "top.txt"], ""]]);
  }, 60_000);

  test("answers in a checkout with 150 checked-out submodules, as Git's own diff does there within Git's time limit", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-no-daemon-"));
    roots.push(root);
    const env = { ...process.env, RERAIL_HOME: join(root, ".rerail-home"), RERAIL_AUTOSTART: "0", RERAIL_EXPERIMENTAL_HEADLESS_SCAN: "1" };

    const scanned = await scannedPaths(await checkoutWithSubmodules(root, 150), env);

    expect(scanned).toEqual([0, ["f0.txt", "sub000"], ""]);
  }, 240_000);
});
