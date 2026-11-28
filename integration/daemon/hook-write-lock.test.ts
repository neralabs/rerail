/**
 * The Claude Code hook against a daemon of this build while another process holds the Rerail database's write lock,
 * as `rerail reset`, `rerail scrub` or a newer Rerail's migration does for seconds: the hook's answers do not wait for
 * the lock, the steer still reaches the agent, a session started meanwhile opens with its note, a session's end, a
 * dashboard save and a save of a checkout's scan settings answer without holding the other sessions' hooks, and every
 * record the daemon took while the lock was held is in the database once it is released.
 */
import { execFile, fork, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { setTimeout as delay } from "node:timers/promises";
import { promisify } from "node:util";
import Database from "better-sqlite3";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { ENTRY_POINTS } from "../../src/engine/index.js";
import { DAEMON_STATE_FILE } from "../../src/runtime/index.js";
import { removeFolder } from "../../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../../test/support/repository-rules.js";

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
  const branches = Array.from({ length: 48 }, (_, index) => `    if x == ${index}:\n        score += ${index + 1}\n`).join("");
  return `def ${name}(a, b, x):\n    score = 0\n${branches}    if a and b:\n        score += 8\n    return score\n`;
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

/** Whether the database holds `sessionId` now as open and claimed: its record not ended, and its owner row active. */
function openAndClaimed(sessionId: string): boolean {
  const database = new Database(join(process.env.RERAIL_HOME!, "rerail.sqlite"), { readonly: true });
  try {
    const row = database.prepare(`
      SELECT sessions.ended_at AS endedAt, owners.active AS active FROM sessions
      JOIN policy_session_owners AS owners ON owners.session_id = sessions.session_id WHERE sessions.session_id = ?
    `).get(sessionId) as { endedAt: number | null; active: number } | undefined;
    return row !== undefined && row.endedAt === null && row.active === 1;
  } finally {
    database.close();
  }
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
    // The holder gives the lock back only once the clean turn's Stop has answered, so every hook of both turns answers
    // while it is held. A hook whose daemon waited for the lock on its thread would wait out SQLite's busy timeout and
    // fail with the lock's error on stderr; one that waited until the lock is free would never answer.
    const lock = await holdWriteLock(join(process.env.RERAIL_HOME!, "rerail.sqlite"));
    let held;
    let clean;
    let auditedWhileHeld;
    try {
      held = await editTurn(sessionId, join(repo, "src", "app", "held_edit.py"));
      clean = await editTurn(sessionId, join(repo, "src", "app", "clean_edit.py"), simple);
      auditedWhileHeld = auditedFiles(sessionId);
    } finally {
      await lock.release();
    }
    const said = (answer: HookAnswer): string => `${String(answer.ms)} ms${answer.stderr === "" ? "" : ` (stderr: ${answer.stderr.trim()})`}`;
    const times = (turn: typeof free): string => `PreToolUse ${said(turn.pre)}, PostToolUse ${said(turn.post)}, Stop ${said(turn.stop)}`;
    const report = `lock free: ${times(free)}; lock held, rule broken: ${times(held)}; lock held, clean edit: ${times(clean)}`;
    expect(contextOf(free.post), report).toContain(`[${RULE}]`);
    // The steer reaches the agent while the lock is held, and its hook hands the steer's receipt to the daemon, which
    // records it once the lock is given back; a receipt neither taken nor recorded fails the hook, which says so on stderr.
    expect(contextOf(held.post), report).toContain(`[${RULE}]`);
    expect(contextOf(clean.post), report).not.toContain(`[${RULE}]`);
    for (const answer of [held.pre, held.post, held.stop, clean.pre, clean.post, clean.stop]) expect(answer.stderr, report).toBe("");
    // The held edit's audit waits in the daemon's record writer while the lock is held.
    expect(auditedWhileHeld, report).not.toContain("held_edit.py");
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

  // Before, the start made the session's waiting row on the daemon's thread, waiting out the 5 s bound for the lock,
  // then its owner claim failed with the lock's error: the hook answered `{}` after 5 s and the agent never got the note.
  it("opens a session started while the lock is held with its note at once, and claims it once the lock is released", async () => {
    const start = (sessionId: string): Promise<HookAnswer> =>
      hook({ session_id: sessionId, cwd: repo, hook_event_name: "SessionStart", source: "startup" }, repo);
    // A start with the lock free: the checkout's stores are open in the daemon, as a person's earlier session leaves them.
    const free = await start("started-free");
    const lock = await holdWriteLock(join(process.env.RERAIL_HOME!, "rerail.sqlite"));
    let held;
    let claimedWhileHeld;
    try {
      held = await start("started-held");
      claimedWhileHeld = openAndClaimed("started-held");
    } finally {
      await lock.release();
    }
    const report = `lock free: ${String(free.ms)} ms; lock held: ${String(held.ms)} ms, stderr: ${held.stderr}`;
    expect(contextOf(free), report).toMatch(/Rerail is on in this repo/u);
    // Answered while the holder still holds the lock, which it gives back only after this answer.
    expect(contextOf(held), report).toMatch(/Rerail is on in this repo/u);
    expect(held.stderr, report).toBe("");
    // The session's row and its claim wait in the daemon's record writer until its lock wait finds the lock given back,
    // which another process learns only from the database.
    expect(claimedWhileHeld, report).toBe(false);
    await vi.waitFor(() => expect(openAndClaimed("started-held")).toBe(true), { timeout: 10_000, interval: 20 });
  }, 60_000);

  // Before, the end interrupted the session's scans on a connection that waited for the lock on the daemon's thread, up
  // to 5 s while every session's hooks waited too, then failed: the hook wrote the lock's error on stderr, and the
  // session's record stayed open, owned by a daemon that had let it go.
  it("ends a session while the lock is held at once, and ends its record and lets it go once the lock is released", async () => {
    const event = (sessionId: string, input: Record<string, unknown>): Promise<HookAnswer> => hook({ session_id: sessionId, cwd: repo, ...input }, repo);
    for (const sessionId of ["ended-free", "ended-held"]) await event(sessionId, { hook_event_name: "SessionStart", source: "startup" });
    const end = (sessionId: string): Promise<HookAnswer> => event(sessionId, { hook_event_name: "SessionEnd", reason: "prompt_input_exit" });
    const free = await end("ended-free");
    const lock = await holdWriteLock(join(process.env.RERAIL_HOME!, "rerail.sqlite"));
    let held;
    let endedWhileHeld;
    try {
      held = await end("ended-held");
      endedWhileHeld = endedAndLetGo("ended-held");
    } finally {
      await lock.release();
    }
    const report = `lock free: ${String(free.ms)} ms; lock held: ${String(held.ms)} ms, stderr: ${held.stderr}`;
    // Answered while the holder still holds the lock, which it gives back only after this answer.
    expect(held.stderr, report).toBe("");
    // The end's writes wait in the daemon's record writer until its lock wait finds the lock given back.
    expect(endedWhileHeld, report).toBe(false);
    await vi.waitFor(() => expect(endedAndLetGo("ended-held")).toBe(true), { timeout: 10_000, interval: 20 });
  }, 60_000);

  // Before, the save waited for the lock on the daemon's thread up to 5 s, the hooks of every session waiting with it,
  // then failed: the dashboard heard 500 and the change was not made.
  it("saves a rule's mode asked in the dashboard while the lock is held once it is released, and answers a hook meanwhile", async () => {
    const sessionId = "saved-beside";
    await hook({ session_id: sessionId, cwd: repo, hook_event_name: "SessionStart", source: "startup" }, repo);
    const read = (): Promise<HookAnswer> => hook({
      session_id: sessionId, cwd: repo, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: `toolu_read_${String(performance.now())}`,
      tool_input: { file_path: join(repo, "src", "app", "__init__.py") },
    }, repo);
    const free = await read();
    const lock = await holdWriteLock(join(process.env.RERAIL_HOME!, "rerail.sqlite"));
    let saving;
    let held;
    const save = { answered: false };
    let answeredBeforeSave;
    try {
      saving = saveRuleMode("log").finally(() => { save.answered = true; });
      held = await read();
      answeredBeforeSave = !save.answered;
    } finally {
      await lock.release();
    }
    const saved = await saving;
    const report = `hook with the lock free: ${String(free.ms)} ms; beside the held save: ${String(held.ms)} ms, stderr: ${held.stderr}; save: ${saved.status} ${saved.body}`;
    expect(saved.status, report).toBe(200);
    expect(JSON.parse(saved.body), report).toMatchObject({ settings: [{ mode: "log" }] });
    expect(held.stderr, report).toBe("");
    // The hook is answered while the save still waits for the lock, which the holder gives back only after that answer.
    expect(answeredBeforeSave, report).toBe(true);
    // The rule back to its start, as the other cases read it.
    expect((await saveRuleMode(null)).status).toBe(200);
  }, 60_000);

  // Before, the save read the checkout's scan settings and then wrote them in one transaction, whose write SQLite fails
  // at once while another process holds the lock, with no busy wait: `rerail settings` exited 1 with "internal" and the
  // API answered 500, the change not made.
  it("saves a checkout's scan mode asked with rerail settings, and a scope asked of the API, while the lock is held 2 s, once it is released, and answers a hook meanwhile", async () => {
    const { saver, read } = await scanSettingsSessions("scan-saved");
    const free = await read();
    const lock = await holdWriteLock(join(process.env.RERAIL_HOME!, "rerail.sqlite"));
    const heldAt = performance.now();
    let saves;
    let held;
    let waitingAtHook;
    try {
      saves = followed(scanSettingsCli(saver, ["--mode", "log"]), scanSettingsApi("PATCH", saver, { scope: "repository" }));
      held = await read();
      waitingAtHook = saves.waiting();
      // The issue's hold: 2 s, well inside SQLite's 5 s busy wait.
      await delay(Math.max(0, 2_000 - (performance.now() - heldAt)));
    } finally {
      await lock.release();
    }
    const [cli, api] = await saves.answers;
    const report = `hook with the lock free: ${String(free.ms)} ms; beside the held saves: ${String(held.ms)} ms; `
      + `rerail settings: exit ${String(cli.code)} after ${String(cli.ms)} ms, ${cli.stdout}${cli.stderr}; PATCH: ${String(api.status)} ${api.body}`;
    expect(cli.code, report).toBe(0);
    expect(cli.stdout, report).toMatch(/\/log, /u);
    expect(api.status, report).toBe(200);
    expect(await savedScanSettings(saver), report).toMatchObject({ scope: "repository", mode: "log" });
    // The hook is answered while both saves still wait for the lock, and says nothing on stderr.
    expect(waitingAtHook, report).toBe(true);
    expect(held.stderr, report).toBe("");
    expect((await scanSettingsApi("PATCH", saver, { scope: "ai-output", mode: "steer" })).status).toBe(200);
  }, 60_000);

  // Before, the save failed at once with "internal" whether the lock was given back in a moment or never.
  it("refuses a scan settings save with 503 database-busy, which rerail settings prints, once the lock stays held past SQLite's 5 s, and answers a hook meanwhile", async () => {
    const { saver, read } = await scanSettingsSessions("scan-busy");
    const before = await savedScanSettings(saver);
    const free = await read();
    const lock = await holdWriteLock(join(process.env.RERAIL_HOME!, "rerail.sqlite"));
    let answers;
    let held;
    let waitingAtHook;
    try {
      const saves = followed(scanSettingsCli(saver, ["--mode", "log"]), scanSettingsApi("PATCH", saver, { mode: "log" }));
      held = await read();
      waitingAtHook = saves.waiting();
      answers = await saves.answers;
    } finally {
      await lock.release();
    }
    const [cli, api] = answers;
    const report = `hook with the lock free: ${String(free.ms)} ms; beside the held saves: ${String(held.ms)} ms; `
      + `rerail settings: exit ${String(cli.code)} after ${String(cli.ms)} ms, ${cli.stdout}${cli.stderr}; PATCH: ${String(api.status)} ${api.body}`;
    const busy = "the change was not made: another process held the Rerail database's write lock for 5 s";
    expect(api.status, report).toBe(503);
    expect(JSON.parse(api.body), report).toMatchObject({ error: "database-busy", message: expect.stringContaining(busy) });
    expect(cli.code, report).toBe(1);
    expect(cli.stderr, report).toContain(`rerail: database-busy: ${busy}`);
    expect(await savedScanSettings(saver), report).toEqual(before);
    // The hook is answered while both saves still wait for the lock, and says nothing on stderr.
    expect(waitingAtHook, report).toBe(true);
    expect(held.stderr, report).toBe("");
  }, 60_000);
});

/**
 * Two sessions started in the checkout: `saver`, whose checkout's scan settings a case saves, and another whose
 * `PreToolUse` hook (`read`) a case times while the save waits.
 */
async function scanSettingsSessions(name: string): Promise<{ saver: string; read: () => Promise<HookAnswer> }> {
  const [saver, other] = [`${name}-saver`, `${name}-other`];
  for (const sessionId of [saver, other]) await hook({ session_id: sessionId, cwd: repo, hook_event_name: "SessionStart", source: "startup" }, repo);
  const read = (): Promise<HookAnswer> => hook({
    session_id: other, cwd: repo, hook_event_name: "PreToolUse", tool_name: "Read", tool_use_id: `toolu_read_${String(performance.now())}`,
    tool_input: { file_path: join(repo, "src", "app", "__init__.py") },
  }, repo);
  return { saver, read };
}

/** `cli` and `api` with whether both still wait for their answers: none of them rejects. */
function followed<A, B>(cli: Promise<A>, api: Promise<B>): { answers: Promise<[A, B]>; waiting(): boolean } {
  let answered = 0;
  const count = (): void => { answered += 1; };
  for (const answer of [cli, api]) void answer.then(count, count);
  return { answers: Promise.all([cli, api]), waiting: () => answered === 0 };
}

interface CliRun {
  code: number | null;
  stdout: string;
  stderr: string;
  ms: number;
}

/** `rerail settings --session <sessionId>` with `args`, in the checkout, in its own process, however it exits. */
async function scanSettingsCli(sessionId: string, args: readonly string[]): Promise<CliRun> {
  const started = performance.now();
  const child = spawn(process.execPath, [CLI, "settings", "--session", sessionId, ...args], {
    cwd: repo, env: environment(), stdio: ["ignore", "pipe", "pipe"], windowsHide: true,
  });
  const [stdout, stderr] = await Promise.all([text(child.stdout), text(child.stderr), once(child, "close")]);
  return { code: child.exitCode, stdout, stderr, ms: Math.round(performance.now() - started) };
}

/** A request to the session's scan settings (`/api/sessions/{id}/audit-settings`), as the daemon answers it. */
async function scanSettingsApi(method: "GET" | "PATCH", sessionId: string, body?: Record<string, unknown>): Promise<{ status: number; body: string }> {
  return await daemonRequest(method, `/api/sessions/${sessionId}/audit-settings`, body);
}

/** The scan settings the daemon reads now for the checkout of `sessionId`. */
async function savedScanSettings(sessionId: string): Promise<unknown> {
  const read = await scanSettingsApi("GET", sessionId);
  return (JSON.parse(read.body) as { settings: unknown }).settings;
}

/** Whether the database holds `sessionId` now as ended and let go: its record ended, and its owner row not active. */
function endedAndLetGo(sessionId: string): boolean {
  const database = new Database(join(process.env.RERAIL_HOME!, "rerail.sqlite"), { readonly: true });
  try {
    const row = database.prepare(`
      SELECT sessions.ended_at AS endedAt, owners.active AS active FROM sessions
      JOIN policy_session_owners AS owners ON owners.session_id = sessions.session_id WHERE sessions.session_id = ?
    `).get(sessionId) as { endedAt: number | null; active: number } | undefined;
    return row !== undefined && row.endedAt !== null && row.active === 0;
  } finally {
    database.close();
  }
}

/** The dashboard's save of `RULE`'s mode for the whole repository (`PUT /api/repository/settings`), as the daemon answers it. */
async function saveRuleMode(mode: "log" | null): Promise<{ status: number; body: string }> {
  return await daemonRequest("PUT", "/api/repository/settings", { checkout: repo, owner: "repository", targets: [{ kind: "rule", id: RULE }], mode });
}

/** One request to the daemon with the person's access token, as the dashboard sends it, and its answer. */
async function daemonRequest(method: "GET" | "PATCH" | "PUT", path: string, body?: Record<string, unknown>): Promise<{ status: number; body: string }> {
  const state = JSON.parse(await readFile(join(process.env.RERAIL_HOME!, DAEMON_STATE_FILE), "utf8")) as { url: string; accessToken: string };
  const response = await fetch(new URL(path, state.url), {
    method,
    headers: { "content-type": "application/json", "x-rerail-access": state.accessToken },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  return { status: response.status, body: await response.text() };
}
