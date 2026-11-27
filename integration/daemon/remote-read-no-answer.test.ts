/**
 * A checkout Rerail has not recorded is named by its remote, which the daemon reads with Git (`git ls-remote --get-url
 * origin`, then `git remote`). When Git gives no answer to that read once, the next read of a daemon of this build asks
 * Git again and names the repository in the dashboard (GET /api/repository) and the status line (GET /api/presentation),
 * with nothing written to the checkout's Git config and no restart.
 *
 * The Git that gives no answer ends as one that could not start: on Windows with status 0xC0000142, elsewhere with the
 * loader's line and exit 127. On Linux and macOS it is a script named `git` first on the daemon's `PATH`, which Rerail
 * finds as its Git and which runs the machine's Git for every other ask. On Windows Rerail runs only a `git.exe` and
 * starts Git through `node:child_process`, so a module the daemon loads before its entry (`--import`) starts the failing
 * process in place of those asks.
 */
import { execFile, execFileSync, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { chmod, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
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
const LOADER_LINE = "git: error while loading shared libraries: libpcre2-8.so.0: cannot open shared object file: No such file or directory";

/** The stand-in elsewhere than Windows: `__STATE__` and `__GIT__` are put in when it is written. */
const SCRIPT = String.raw`#!/bin/sh
state='__STATE__'
if [ -e "$state/arm" ]; then
  case " $* " in
    *" ls-remote --get-url origin "|*" remote ")
      echo failed >> "$state/log"
      echo "${LOADER_LINE}" >&2
      exit 127
      ;;
  esac
fi
exec '__GIT__' "$@"
`;

/** The stand-in on Windows, loaded into the daemon with `--import`: it reads its folder from `REMOTE_NO_ANSWER_STATE`. */
const PRELOAD = String.raw`
import childProcess from "node:child_process";
import { appendFileSync, existsSync } from "node:fs";
import { syncBuiltinESMExports } from "node:module";
import { join } from "node:path";
const state = process.env.REMOTE_NO_ANSWER_STATE;
function cannotStart(args) {
  if (!Array.isArray(args) || !existsSync(join(state, "arm"))) return false;
  const asked = " " + args.join(" ");
  return asked.endsWith(" ls-remote --get-url origin") || asked.endsWith(" remote");
}
const failing = [process.execPath, ["-e", "process.exit(3221225794)"]];
if (state !== undefined) {
  const { spawn, spawnSync } = childProcess;
  childProcess.spawn = function (file, args, ...rest) {
    if (!cannotStart(args)) return spawn.call(this, file, args, ...rest);
    appendFileSync(join(state, "log"), "failed\n");
    return spawn.call(this, ...failing, ...rest);
  };
  childProcess.spawnSync = function (file, args, ...rest) {
    if (!cannotStart(args)) return spawnSync.call(this, file, args, ...rest);
    appendFileSync(join(state, "log"), "failed\n");
    return spawnSync.call(this, ...failing, ...rest);
  };
  syncBuiltinESMExports();
}
`;

let root: string;
let home: string;
let state: string;
let daemon: ChildProcess | undefined;

const environment = (): NodeJS.ProcessEnv => ({ ...process.env, RERAIL_HOME: home, BROWSER: "none" });

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

/** What the daemon's command line puts before its entry, and what its environment adds, for the stand-in in `state`. */
async function standIn(): Promise<{ readonly args: readonly string[]; readonly env: NodeJS.ProcessEnv }> {
  if (process.platform === "win32") return { args: ["--import", `data:text/javascript,${encodeURIComponent(PRELOAD)}`], env: { REMOTE_NO_ANSWER_STATE: state } };
  const bin = join(state, "bin");
  await mkdir(bin);
  const machineGit = execFileSync("sh", ["-c", "command -v git"], { encoding: "utf8" }).trim();
  if ([state, machineGit].some(path => path.includes("'"))) throw new Error(`the Git stand-in cannot quote ${state} or ${machineGit}`);
  await writeFile(join(bin, "git"), SCRIPT.replace("__GIT__", machineGit).replace("__STATE__", state));
  await chmod(join(bin, "git"), 0o755);
  return { args: [], env: { PATH: `${bin}:${process.env.PATH ?? ""}` } };
}

async function startDaemon(): Promise<void> {
  const { args, env } = await standIn();
  const child = spawn(process.execPath, [...args, join(PACKAGE_ROOT, ENTRY_POINTS.daemon), "--port", "0", "--state-file", join(home, DAEMON_STATE_FILE)], {
    env: { ...environment(), ...env }, stdio: ["ignore", "pipe", "ignore"], windowsHide: true,
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
  await promisify(execFile)(process.execPath, [CLI, "daemon", "stop"], { cwd: root, env: environment(), windowsHide: true }).catch(() => child.kill());
  await closed;
  daemon = undefined;
}

/** A read of the daemon's API, with the access token its state file records: its status, and the body of a 200. */
async function read(path: string, query: Record<string, string>): Promise<{ readonly status: number; readonly body: unknown }> {
  const daemonState = JSON.parse(await readFile(join(home, DAEMON_STATE_FILE), "utf8")) as Pick<DaemonState, "url" | "accessToken">;
  const response = await fetch(new URL(`${path}?${new URLSearchParams(query)}`, daemonState.url), { headers: { "x-rerail-access": daemonState.accessToken! } });
  return { status: response.status, body: response.status === 200 ? await response.json() : undefined };
}

/** The repository the dashboard names for `checkout` (null on its 404), and the repository and state the status line gives it. */
async function shown(checkout: string) {
  const page = await read("/api/repository", { checkout });
  const presentation = (await read("/api/presentation", { repository: checkout, host: "claude-code" })).body as {
    scope: { repositoryKey: string } | null;
    repositoryState: string;
  };
  return {
    dashboard: page.status === 200 ? (page.body as { repository: { repositoryKey: string } }).repository.repositoryKey : null,
    statusLine: presentation.scope?.repositoryKey ?? null,
    statusLineState: presentation.repositoryState,
  };
}

/** How many asks the stand-in failed. */
async function failedAsks(): Promise<number> {
  return (await readFile(join(state, "log"), "utf8").catch(() => "")).split("\n").filter(Boolean).length;
}

let alpha: string;
let checkout: string;

beforeAll(async () => {
  root = await realpath(await mkdtemp(join(tmpdir(), "rerail-remote-no-answer-")));
  home = join(root, "home");
  state = join(root, "stand-in");
  await mkdir(state);
  // Alpha is set up and started; the checkout, cloned from alpha, is recorded by none.
  alpha = setUpTestRepository(await repository("alpha", ALPHA), {}, join(home, "rerail.sqlite"));
  checkout = await repository("x", ALPHA);
  await startDaemon();
}, 120_000);

afterAll(async () => {
  await stopDaemon();
  await removeFolder(root);
}, 60_000);

describe("a checkout Rerail has not recorded, whose remote read Git gave no answer to once", () => {
  it("names the repository its remote names at the next read, with no config write and no restart", async () => {
    await writeFile(join(state, "arm"), "");
    expect(await shown(checkout)).toEqual({ dashboard: null, statusLine: null, statusLineState: "stopped" });
    await rm(join(state, "arm"));
    expect(await failedAsks()).toBeGreaterThan(0);

    expect(await shown(checkout)).toEqual({ dashboard: alpha, statusLine: alpha, statusLineState: "started" });
  }, 60_000);
});
