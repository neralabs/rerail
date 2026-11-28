/**
 * A plain folder of a started repository's checkout, which a daemon of this build has read, is made a repository of its
 * own (`git init`) whose remote names a repository Rerail has set up and stopped. At its next read the dashboard
 * (GET /api/repository) and the status line (GET /api/presentation) name the new repository and its state, with no
 * restart, as `rerail status` does in its own process; and a Claude Code session started there, through the built hook,
 * is told what a session in a folder made a repository the same way, never read before, is told.
 */
import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ENTRY_POINTS } from "../../src/engine/index.js";
import { DAEMON_STATE_FILE, type DaemonState } from "../../src/runtime/index.js";
import { removeFolder } from "../../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../../test/support/repository-rules.js";

const PACKAGE_ROOT = resolve(".");
const CLI = join(PACKAGE_ROOT, "dist", "cli", "cli.js");
const HOOK = [join(PACKAGE_ROOT, "dist", "harness", "hook-launcher.js"), process.execPath, join(PACKAGE_ROOT, "dist", "hosts", "claude", "claude-hook.js")];
const APP = "https://example.invalid/team/app.git";
const TOOLS = "https://example.invalid/team/tools.git";

let root: string;
let home: string;
let daemon: ChildProcess | undefined;

const environment = (extra: NodeJS.ProcessEnv = {}): NodeJS.ProcessEnv => ({ ...process.env, RERAIL_HOME: home, BROWSER: "none", ...extra });

async function rerail(args: readonly string[], cwd: string): Promise<string> {
  const { stdout } = await promisify(execFile)(process.execPath, [CLI, ...args], { cwd, env: environment(), windowsHide: true });
  return stdout;
}

async function git(args: readonly string[], cwd: string): Promise<void> {
  await promisify(execFile)("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd, windowsHide: true });
}

/** `path` made a repository with one commit and `origin` at `remote`, as `git init` makes one in a folder that has files. */
async function repositoryAt(path: string, remote: string): Promise<string> {
  await git(["init", "-q", "-b", "main"], path);
  await git(["remote", "add", "origin", remote], path);
  await git(["add", "."], path);
  await git(["commit", "-q", "--allow-empty", "-m", "first"], path);
  return path;
}

/** A plain folder at `path` with one file. */
async function folderAt(path: string): Promise<string> {
  await mkdir(path, { recursive: true });
  await writeFile(join(path, "notes.txt"), "notes\n");
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

/** The repository the dashboard and the status line name for `folder`, the state the status line gives it, and `rerail status` there. */
async function shown(folder: string) {
  const page = await read("/api/repository", { checkout: folder }) as { repository: { repositoryKey: string } };
  const presentation = await read("/api/presentation", { repository: folder, host: "claude-code" }) as {
    scope: { repositoryKey: string } | null;
    repositoryState: string;
  };
  return {
    dashboard: page.repository.repositoryKey,
    statusLine: presentation.scope?.repositoryKey ?? null,
    statusLineState: presentation.repositoryState,
    status: (await rerail(["status"], folder)).split("\n")[0],
  };
}

/**
 * What a Claude Code session started in `folder` is told, through the built hook as Claude Code runs it: the note its
 * start answers with, the session's own id put back as `<session>` so two sessions' notes compare.
 */
async function toldAtStart(folder: string, sessionId: string): Promise<string> {
  const child = spawn(process.execPath, HOOK, {
    cwd: folder, env: environment({ CLAUDE_PROJECT_DIR: folder }), stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
  });
  child.stdin.end(JSON.stringify({ session_id: sessionId, cwd: folder, hook_event_name: "SessionStart", source: "startup" }));
  const [stdout] = await Promise.all([text(child.stdout), text(child.stderr), once(child, "close")]);
  const output = (stdout.trim() === "" ? {} : JSON.parse(stdout)) as { hookSpecificOutput?: { additionalContext?: string } };
  return (output.hookSpecificOutput?.additionalContext ?? "").replaceAll(sessionId, "<session>");
}

let app: string;
let tools: string;

beforeAll(async () => {
  root = await realpath(await mkdtemp(join(tmpdir(), "rerail-subfolder-repository-")));
  home = join(root, "home");
  const database = join(home, "rerail.sqlite");
  // App is set up and started; tools, a clone elsewhere, is set up and stopped.
  app = setUpTestRepository(await repositoryAt(await folderAt(join(root, "app")), APP), {}, database);
  tools = setUpTestRepository(await repositoryAt(await folderAt(join(root, "tools")), TOOLS), { start: false }, database);
  await startDaemon();
}, 120_000);

afterAll(async () => {
  await stopDaemon();
  await removeFolder(root);
}, 60_000);

describe("a folder of a checkout the daemon has read, made a repository of its own while the daemon runs", () => {
  it("names the new repository and its state at the next read, and a session started there is told what a new repository's session is told", async () => {
    const folder = await folderAt(join(root, "app", "lib"));
    const neverRead = await folderAt(join(root, "app", "fresh"));
    expect(await shown(folder)).toEqual({ dashboard: app, statusLine: app, statusLineState: "started", status: "Rerail is on in app." });
    expect(await toldAtStart(folder, "before")).toMatch(/^Rerail is on in this repo/u);

    await repositoryAt(folder, TOOLS);
    await repositoryAt(neverRead, TOOLS);

    expect(await shown(folder)).toEqual({
      dashboard: tools, statusLine: tools, statusLineState: "stopped", status: "Rerail is stopped in lib. Start it with rerail start.",
    });
    expect(await toldAtStart(folder, "after")).toBe(await toldAtStart(neverRead, "fresh"));
  }, 90_000);
});
