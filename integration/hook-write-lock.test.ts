/**
 * The Claude Code hook against a daemon of this build while another process holds the Rerail database's write lock,
 * as `rerail reset`, `rerail scrub` or a newer Rerail's migration does for seconds: the hook's answers do not wait for
 * the lock, the steer still reaches the agent, and every record the daemon took while the lock was held is in the
 * database once it is released.
 */
import { execFile, fork, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { promisify } from "node:util";
import Database from "better-sqlite3";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { ENTRY_POINTS } from "../src/engine/index.js";
import { DAEMON_STATE_FILE } from "../src/runtime/index.js";
import { removeFolder } from "../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../test/support/repository-rules.js";

const PACKAGE_ROOT = resolve(".");
const CLI = join(PACKAGE_ROOT, "dist", "cli", "cli.js");
const HOOK = [join(PACKAGE_ROOT, "dist", "harness", "hook-launcher.js"), process.execPath, join(PACKAGE_ROOT, "dist", "hosts", "claude", "claude-hook.js")];
const RULE = "complexity-cyclomatic-python";
const PACK = "@neralabs/rerail-pack-complexity";

let root: string;
let repo: string;
let daemon: ChildProcess | undefined;

const environment = (extra: NodeJS.ProcessEnv = {}): NodeJS.ProcessEnv => ({ ...process.env, BROWSER: "none", ...extra });

async function rerail(args: readonly string[], cwd = repo): Promise<string> {
  const { stdout } = await promisify(execFile)(process.execPath, [CLI, ...args], { cwd, env: environment(), windowsHide: true });
  return stdout;
}

async function git(args: readonly string[], cwd: string): Promise<void> {
  await promisify(execFile)("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd, windowsHide: true });
}

async function startDaemon(): Promise<void> {
  const home = process.env.RERAIL_HOME!;
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
  await rerail(["daemon", "stop"]).catch(() => child.kill());
  await closed;
  daemon = undefined;
}

interface HookAnswer {
  output: { hookSpecificOutput?: { additionalContext?: string } };
  stderr: string;
  ms: number;
}

/** One hook event, as Claude Code sends it, in its own process, with the time it took to answer. */
async function hook(input: Record<string, unknown>, projectDir: string): Promise<HookAnswer> {
  const started = performance.now();
  const child = spawn(process.execPath, HOOK, {
    cwd: projectDir, env: environment({ CLAUDE_PROJECT_DIR: projectDir }), stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
  });
  child.stdin.end(JSON.stringify(input));
  const [stdout, stderr] = await Promise.all([text(child.stdout), text(child.stderr), once(child, "close")]);
  return { output: JSON.parse(stdout) as HookAnswer["output"], stderr, ms: Math.round(performance.now() - started) };
}

const contextOf = (answer: HookAnswer): string => answer.output.hookSpecificOutput?.additionalContext ?? "";

function tooComplex(name: string): string {
  const branches = ["a", "b", "c", "d", "e", "f", "g"].map((flag, index) => `    if ${flag}:\n        score += ${index + 1}\n`).join("");
  return `def ${name}(a, b, c, d, e, f, g):\n    score = 0\n${branches}    if a and b:\n        score += 8\n    return score\n`;
}

/** A Python function well under the rule's ceiling. */
function simple(name: string): string {
  return `def ${name}(a):\n    return a + 1\n`;
}

/** PreToolUse, the write of `content`, PostToolUse and Stop of one edit, each hook's answer kept. */
async function editTurn(sessionId: string, file: string, content: (name: string) => string = tooComplex): Promise<{ pre: HookAnswer; post: HookAnswer; stop: HookAnswer }> {
  const name = basename(file, ".py");
  const text = content(name);
  const common = { session_id: sessionId, cwd: repo, tool_name: "Write", tool_use_id: `toolu_${name}`, tool_input: { file_path: file, content: text } };
  const pre = await hook({ ...common, hook_event_name: "PreToolUse" }, repo);
  await mkdir(dirname(file), { recursive: true });
  await writeFile(file, text);
  const post = await hook({ ...common, hook_event_name: "PostToolUse", tool_response: { type: "create", filePath: file, content: text } }, repo);
  const stop = await hook({ session_id: sessionId, cwd: repo, hook_event_name: "Stop", prompt_id: "prompt-1", last_assistant_message: "Done." }, repo);
  return { pre, post, stop };
}

/**
 * Edit turns that break the rule until one is steered, at most five: a session's first boundaries are answered while
 * its rules still compile (by design), so the timed turns start only once the session checks edits.
 */
async function untilRulesCheck(sessionId: string): Promise<void> {
  for (let turn = 0; turn < 5; turn += 1) {
    const answers = await editTurn(sessionId, join(repo, "src", "app", `warm_${String(turn)}.py`));
    if (Object.values(answers).some(answer => contextOf(answer).includes(`[${RULE}]`))) return;
  }
  throw new Error("no edit was steered in five turns");
}

/** A process of its own holding the database's write lock (`BEGIN IMMEDIATE`) until released. */
async function holdWriteLock(path: string): Promise<{ release(): Promise<void> }> {
  const script = join(root, "hold-write-lock.cjs");
  await writeFile(script, [
    "const Database = require(process.argv[2]);",
    "const database = new Database(process.argv[3]);",
    "database.exec('BEGIN IMMEDIATE');",
    "process.on('message', () => { database.exec('COMMIT'); database.close(); process.send('released'); });",
    "process.send('held');",
  ].join("\n"));
  const child = fork(script, [createRequire(import.meta.url).resolve("better-sqlite3"), path], { stdio: ["ignore", "inherit", "inherit", "ipc"] });
  await once(child, "message");
  return {
    async release() {
      const released = once(child, "message");
      child.send("release");
      await released;
      // Its IPC channel keeps the holder alive: closing it lets the process end.
      const exited = once(child, "exit");
      child.disconnect();
      await exited;
    },
  };
}

/** The files with a violation of the rule in the audits the database holds now for `sessionId`. */
function auditedFiles(sessionId: string): string[] {
  const database = new Database(join(process.env.RERAIL_HOME!, "rerail.sqlite"), { readonly: true });
  try {
    const rows = database.prepare("SELECT violations_json AS violations FROM audits WHERE session_id = ?").all(sessionId) as Array<{ violations: string }>;
    return rows.flatMap(row => (JSON.parse(row.violations) as Array<{ ruleId: string; path: string }>).filter(violation => violation.ruleId === RULE).map(violation => basename(violation.path)));
  } finally {
    database.close();
  }
}

beforeAll(async () => {
  root = await realpath(await mkdtemp(join(tmpdir(), "rerail-hook-write-lock-")));
  repo = join(root, "app");
  process.env.RERAIL_HOME = join(root, "home");
  await mkdir(join(repo, "src", "app"), { recursive: true });
  await writeFile(join(repo, "src", "app", "__init__.py"), "");
  await git(["init", "-q", "-b", "main"], repo);
  await git(["add", "."], repo);
  await git(["commit", "-q", "-m", "base"], repo);
  await rerail(["packs", "add", join(PACKAGE_ROOT, "packs", "complexity")], root);
  await rerail(["rules", "approve", PACK], root);
  setUpTestRepository(repo, { packs: ["complexity"] });
  await startDaemon();
}, 120_000);

afterAll(async () => {
  await stopDaemon();
  await removeFolder(root);
}, 60_000);

describe("the Claude Code hook while another process holds the database's write lock", () => {
  it("answers each hook without waiting for the lock, steers, and records every event once the lock is released", async () => {
    const sessionId = "write-lock-session";
    const start = await hook({ session_id: sessionId, cwd: repo, hook_event_name: "SessionStart", source: "startup" }, repo);
    expect(contextOf(start)).toMatch(/Rerail is on in this repo/u);
    await hook({ session_id: sessionId, cwd: repo, hook_event_name: "UserPromptSubmit", prompt_id: "prompt-1", prompt: "Add it." }, repo);
    await untilRulesCheck(sessionId);
    const free = await editTurn(sessionId, join(repo, "src", "app", "free_edit.py"));
    const lock = await holdWriteLock(join(process.env.RERAIL_HOME!, "rerail.sqlite"));
    let held;
    let clean;
    try {
      held = await editTurn(sessionId, join(repo, "src", "app", "held_edit.py"));
      clean = await editTurn(sessionId, join(repo, "src", "app", "clean_edit.py"), simple);
    } finally {
      await lock.release();
    }
    const times = (turn: typeof free): string => `PreToolUse ${turn.pre.ms} ms, PostToolUse ${turn.post.ms} ms, Stop ${turn.stop.ms} ms`;
    const report = `lock free: ${times(free)}; lock held, rule broken: ${times(held)}; lock held, clean edit: ${times(clean)}`;
    expect(contextOf(free.post), report).toContain(`[${RULE}]`);
    // The steer reaches the agent while the lock is held. Its hook then records the steer's receipt itself, on a
    // connection of its own that waits for the lock up to 5 s; handing the receipt to the daemon removes that wait.
    expect(contextOf(held.post), report).toContain(`[${RULE}]`);
    expect(contextOf(clean.post), report).not.toContain(`[${RULE}]`);
    for (const turn of [held, clean]) {
      expect(turn.pre.ms, report).toBeLessThan(free.pre.ms + 2_000);
      expect(turn.stop.ms, report).toBeLessThan(free.stop.ms + 2_000);
    }
    expect(clean.post.ms, report).toBeLessThan(free.post.ms + 2_000);
    // The records the daemon took while the lock was held: the session's events and the edit's audit with its violation.
    // The daemon makes them once its lock wait finds the lock given back, which another process learns only from the
    // database: the bundle is read once the held edit's audit is there, within twice the waiting write's own 5 s bound.
    await vi.waitFor(() => expect(auditedFiles(sessionId)).toContain("held_edit.py"), { timeout: 10_000, interval: 20 });
    const bundleFile = join(root, "bundle.json");
    await rerail(["support-bundle", "--session", sessionId, "--output", bundleFile]);
    const bundle = JSON.parse(await readFile(bundleFile, "utf8")) as { session: { audits: Array<{ violations: Array<{ ruleId: string; path: string }> }> } };
    const logged = bundle.session.audits.flatMap(audit => audit.violations).filter(violation => violation.ruleId === RULE).map(violation => basename(violation.path));
    expect(new Set(logged.filter(file => !file.startsWith("warm_"))), report).toEqual(new Set(["free_edit.py", "held_edit.py"]));
  }, 180_000);
});
