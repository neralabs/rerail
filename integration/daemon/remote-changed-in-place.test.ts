/**
 * A checkout Rerail has not recorded is named by its remote. After `git remote set-url` there, a daemon of this build
 * that read it before names the new remote's repository at its next read, in the dashboard (GET /api/repository) and
 * the status line (GET /api/presentation), with no restart; `rerail status`, which reads in its own process, agrees.
 */
import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, readFile, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ENTRY_POINTS } from "../../src/engine/index.js";
import { DAEMON_STATE_FILE, type DaemonState } from "../../src/runtime/index.js";
import { removeFolder } from "../../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../../test/support/repository-rules.js";

const PACKAGE_ROOT = resolve(".");
const CLI = join(PACKAGE_ROOT, "dist", "cli", "cli.js");
const ALPHA = "https://example.invalid/team/alpha.git";
const BETA = "https://example.invalid/team/beta.git";

let root: string;
let home: string;
let daemon: ChildProcess | undefined;

const environment = (): NodeJS.ProcessEnv => ({ ...process.env, RERAIL_HOME: home, BROWSER: "none" });

async function rerail(args: readonly string[], cwd: string): Promise<string> {
  const { stdout } = await promisify(execFile)(process.execPath, [CLI, ...args], { cwd, env: environment(), windowsHide: true });
  return stdout;
}

async function git(args: readonly string[], cwd: string): Promise<void> {
  await promisify(execFile)("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd, windowsHide: true });
}

/** A repository at `root/<name>` with one commit and `origin` at `remote`. */
async function repository(name: string, remote: string): Promise<string> {
  const path = join(root, name);
  await git(["init", "-q", "-b", "main", path], root);
  await git(["remote", "add", "origin", remote], path);
  await git(["commit", "-q", "--allow-empty", "-m", "first"], path);
  return path;
}

async function startDaemon(): Promise<void> {
  const child = spawn(process.execPath, [join(PACKAGE_ROOT, ENTRY_POINTS.daemon), "--port", "0", "--state-file", join(home, DAEMON_STATE_FILE)], {
    env: environment(), stdio: ["ignore", "pipe", "ignore"], windowsHide: true,
  });
  daemon = child;
  await Promise.race([
    once(child.stdout!, "data"),
    once(child, "close").then(([code]) => { throw new Error(`daemon exited with code ${String(code)}`); }),
  ]);
  child.stdout!.resume();
}

async function stopDaemon(): Promise<void> {
  const child = daemon;
  if (child === undefined || child.exitCode !== null) return;
  const closed = once(child, "close");
  await rerail(["daemon", "stop"], root).catch(() => child.kill());
  await closed;
  daemon = undefined;
}

/** A read of the daemon's API, with the access token its state file records, as the dashboard and the status line read. */
async function read(path: string, query: Record<string, string>): Promise<unknown> {
  const state = JSON.parse(await readFile(join(home, DAEMON_STATE_FILE), "utf8")) as Pick<DaemonState, "url" | "accessToken">;
  const response = await fetch(new URL(`${path}?${new URLSearchParams(query)}`, state.url), { headers: { "x-rerail-access": state.accessToken! } });
  expect(response.status, path).toBe(200);
  return await response.json();
}

/** The repository the dashboard and the status line name for `checkout`, the state the status line gives it, and `rerail status` there. */
async function shown(checkout: string) {
  const page = await read("/api/repository", { checkout }) as { repository: { repositoryKey: string } };
  const presentation = await read("/api/presentation", { repository: checkout, host: "claude-code" }) as {
    scope: { repositoryKey: string } | null;
    repositoryState: string;
  };
  return {
    dashboard: page.repository.repositoryKey,
    statusLine: presentation.scope?.repositoryKey ?? null,
    statusLineState: presentation.repositoryState,
    status: (await rerail(["status"], checkout)).split("\n")[0],
  };
}

let alpha: string;
let beta: string;
let checkout: string;

beforeAll(async () => {
  root = await realpath(await mkdtemp(join(tmpdir(), "rerail-remote-in-place-")));
  home = join(root, "home");
  const database = join(home, "rerail.sqlite");
  // Alpha is started, beta set up and stopped; the checkout, cloned from alpha, is recorded by neither.
  alpha = setUpTestRepository(await repository("alpha", ALPHA), {}, database);
  beta = setUpTestRepository(await repository("beta", BETA), { start: false }, database);
  checkout = await repository("x", ALPHA);
  await startDaemon();
}, 120_000);

afterAll(async () => {
  await stopDaemon();
  await removeFolder(root);
}, 60_000);

describe("a checkout Rerail has not recorded, whose remote is changed in place while the daemon runs", () => {
  it("names the new remote's repository and its state at the next read, with no restart", async () => {
    expect(await shown(checkout)).toEqual({ dashboard: alpha, statusLine: alpha, statusLineState: "started", status: "Rerail is on in x." });

    await git(["remote", "set-url", "origin", BETA], checkout);

    expect(await shown(checkout)).toEqual({
      dashboard: beta, statusLine: beta, statusLineState: "stopped", status: "Rerail is stopped in x. Start it with rerail start.",
    });
  }, 60_000);
});
