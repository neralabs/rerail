/**
 * Rules fire in a Claude Code session: the built hook, run as Claude Code runs it on Windows
 * (`dist/harness/hook-launcher.js` starting `dist/hosts/claude/claude-hook.js`, one process per event, with the
 * session's `CLAUDE_PROJECT_DIR`), replays a session's events against a daemon of this build in this file's own Rerail
 * home. Elsewhere Claude Code starts `claude-hook.js` through a shell; the launcher, handed the node it runs on,
 * imports the hook into its own process, so the hook runs there as `node claude-hook.js` runs it.
 *
 * The repository runs the complexity pack from this checkout, approved, so a Python function above the cyclomatic
 * ceiling breaks `complexity-cyclomatic-python`. Each session edits such a function in the session's checkout, through
 * a subagent (whose tool calls Claude Code sends under the parent's session id) and in a linked worktree outside the
 * checkout, and the test reads back what a person reads: the steer in the hook's answer, the session's rule count from
 * `rerail session`, and the logged violation from `rerail support-bundle`.
 *
 * Two cases go red on causes that left a person's sessions with "rules unavailable" or edits unchecked: a session opened
 * from VS Code, which spells the drive letter `c:` (Windows), and subagents whose hooks run at once while the daemon is
 * slow to answer. The last case checks that a session's edits are still checked after its daemon restarts. One more
 * writes the file through a Bash command while another process keeps it from being read, from its first byte until the
 * session's record shows that the check at the command's result could not read it: the agent's next command is
 * steered on it.
 */
import { execFile, spawn, type ChildProcess } from "node:child_process";
import Database from "better-sqlite3";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { createServer, request, type Server } from "node:http";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { setTimeout as delay } from "node:timers/promises";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { ENTRY_POINTS } from "../src/engine/index.js";
import { DAEMON_STATE_FILE, ENGINE_CHALLENGE_HEADER, ENGINE_PROOF_PATH, type DaemonState } from "../src/runtime/index.js";
import { removeFolder } from "../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../test/support/repository-rules.js";
import { awaitCodeRulesOpenThrough } from "./support/code-rules-open.js";

const PACKAGE_ROOT = resolve(".");
const CLI = join(PACKAGE_ROOT, "dist", "cli", "cli.js");
const HOOK = [join(PACKAGE_ROOT, "dist", "harness", "hook-launcher.js"), process.execPath, join(PACKAGE_ROOT, "dist", "hosts", "claude", "claude-hook.js")];
const RULE = "complexity-cyclomatic-python";
const PACK = "@neralabs/rerail-pack-complexity";

let root: string;
let repo: string;
let worktree: string;
let daemon: ChildProcess | undefined;
const servers: Server[] = [];

const environment = (extra: NodeJS.ProcessEnv = {}): NodeJS.ProcessEnv => ({ ...process.env, BROWSER: "none", ...extra });

async function rerail(args: readonly string[], cwd = repo): Promise<string> {
  const { stdout } = await promisify(execFile)(process.execPath, [CLI, ...args], { cwd, env: environment(), windowsHide: true });
  return stdout;
}

async function git(args: readonly string[], cwd: string): Promise<void> {
  await promisify(execFile)("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd, windowsHide: true });
}

/** A daemon of this build in the file's Rerail home, in the foreground, answering once its state file is written. */
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

/** Stops the daemon the way `rerail daemon stop` does, which lets it close itself and remove its state file. */
async function stopDaemon(): Promise<void> {
  const child = daemon;
  if (child === undefined || child.exitCode !== null) return;
  const closed = once(child, "close");
  await rerail(["daemon", "stop"]).catch(() => child.kill());
  await closed;
  daemon = undefined;
}

async function daemonState(): Promise<DaemonState> {
  return JSON.parse(await readFile(join(process.env.RERAIL_HOME!, DAEMON_STATE_FILE), "utf8")) as DaemonState;
}

interface HookAnswer {
  output: { hookSpecificOutput?: { additionalContext?: string } };
  stderr: string;
}

/** One hook event, as Claude Code sends it, in its own process. */
async function hook(input: Record<string, unknown>, projectDir: string, extra: NodeJS.ProcessEnv = {}): Promise<HookAnswer> {
  const child = spawn(process.execPath, HOOK, {
    cwd: projectDir, env: environment({ CLAUDE_PROJECT_DIR: projectDir, ...extra }), stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
  });
  child.stdin.end(JSON.stringify(input));
  const [stdout, stderr] = await Promise.all([text(child.stdout), text(child.stderr), once(child, "close")]);
  return { output: JSON.parse(stdout) as HookAnswer["output"], stderr };
}

const contextOf = (answer: HookAnswer): string => answer.output.hookSpecificOutput?.additionalContext ?? "";

/** A Python function at cyclomatic complexity 10, over the pack's ceiling of 6. */
function tooComplex(name: string): string {
  const branches = ["a", "b", "c", "d", "e", "f", "g"].map((flag, index) => `    if ${flag}:\n        score += ${index + 1}\n`).join("");
  return `def ${name}(a, b, c, d, e, f, g):\n    score = 0\n${branches}    if a and b:\n        score += 8\n    return score\n`;
}

/** What a session replays: its id, the folder Claude Code was opened in, and extra hook environment. */
interface Session {
  id: string;
  projectDir: string;
  env?: NodeJS.ProcessEnv;
}

/** A Write of `file` with a function that breaks the rule: PreToolUse, the write, PostToolUse; the PostToolUse answer. */
async function writeTooComplex(session: Session, file: string, agent?: string): Promise<HookAnswer> {
  const name = basename(file, ".py");
  const content = tooComplex(name);
  const subagent = agent === undefined ? {} : { agent_id: agent, agent_type: "general-purpose" };
  const tool = { tool_name: "Write", tool_use_id: `toolu_${name}`, tool_input: { file_path: file, content }, ...subagent };
  const common = { session_id: session.id, cwd: session.projectDir, ...tool };
  const pre = await hook({ ...common, hook_event_name: "PreToolUse" }, session.projectDir, session.env);
  expect(pre.stderr).toBe("");
  await mkdir(dirname(file), { recursive: true });
  await writeFile(file, content);
  return await hook({ ...common, hook_event_name: "PostToolUse", tool_response: { type: "create", filePath: file, content } }, session.projectDir, session.env);
}

async function startSession(session: Session): Promise<void> {
  const common = { session_id: session.id, cwd: session.projectDir };
  const start = await hook({ ...common, hook_event_name: "SessionStart", source: "startup" }, session.projectDir, session.env);
  // The session-start note counts the rules on: the session's rules are available.
  expect(contextOf(start)).toMatch(/Rerail is on in this repo: \d+ rules? (?:is|are) on\./u);
  await hook({ ...common, hook_event_name: "UserPromptSubmit", prompt_id: "prompt-1", prompt: "Add the classifiers." }, session.projectDir, session.env);
  // A session's first start answers its tool calls without code rules while they open (the first index of the
  // checkout, its rules compiled), and the first pass after checks what was edited meanwhile; only an edit made once
  // they run is steered at its own result. So each session waits for them, through Read calls that edit nothing, sent
  // to the daemon itself so a slow engine in front of it does not hold the wait.
  await awaitCodeRulesOpenThrough(session.id, async probe => {
    const read = { ...common, tool_name: "Read", tool_use_id: `toolu_open_probe_${probe}`, tool_input: { file_path: join(session.projectDir, "pyproject.toml") } };
    await hook({ ...read, hook_event_name: "PreToolUse" }, session.projectDir);
    await hook({ ...read, hook_event_name: "PostToolUse", tool_response: { type: "text" } }, session.projectDir);
  });
}

/** The turn's end: `Stop`'s answer. */
async function endTurn(session: Session): Promise<HookAnswer> {
  const common = { session_id: session.id, cwd: session.projectDir };
  return await hook({ ...common, hook_event_name: "Stop", prompt_id: "prompt-1", last_assistant_message: "Done." }, session.projectDir, session.env);
}

/** A Bash command of the session: PreToolUse, what it does (`run`), PostToolUse; both hooks' answers. */
async function runCommand(session: Session, toolUseId: string, run: () => Promise<void>): Promise<readonly HookAnswer[]> {
  const tool = { tool_name: "Bash", tool_use_id: toolUseId, tool_input: { command: "python3 scripts/generate.py" } };
  const common = { session_id: session.id, cwd: session.projectDir, ...tool };
  const pre = await hook({ ...common, hook_event_name: "PreToolUse" }, session.projectDir, session.env);
  expect(pre.stderr).toBe("");
  await run();
  const tool_response = { stdout: "", stderr: "", interrupted: false, isImage: false };
  return [pre, await hook({ ...common, hook_event_name: "PostToolUse", tool_response }, session.projectDir, session.env)];
}

/**
 * Another process that writes `HOLD_TEXT` to a new file at `HOLD_PATH` and keeps it from being read, from its first
 * byte, until its input ends, saying `held` once it holds the written file and `released` once it let go. So no walk
 * can read the file before the check at the command's result, however the daemon's walks fall. On Windows it creates
 * and writes the file through one handle with no sharing, as a program that scans or copies a file may hold it:
 * libuv's UV_FS_O_EXLOCK flag, which `fs.constants` does not name; the file keeps the write's times when it closes.
 * Elsewhere no process can hold a file from reads without changing its times, which the walk reads as a later write:
 * it writes the file in a folder of its own outside the checkout (`HOLD_STAGING`), takes the search permission off that
 * folder and moves it into place, which leaves the file's times as the write set them.
 */
const HOLDER = `
import { chmodSync, closeSync, constants, mkdirSync, openSync, renameSync, statSync, writeFileSync, writeSync } from "node:fs";
import { basename, dirname, join } from "node:path";
const path = process.env.HOLD_PATH;
const folder = dirname(path);
let release;
if (process.platform === "win32") {
  mkdirSync(folder, { recursive: true });
  const fd = openSync(path, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | 0x10000000);
  writeSync(fd, process.env.HOLD_TEXT);
  release = () => closeSync(fd);
} else {
  const staged = join(process.env.HOLD_STAGING, basename(folder));
  mkdirSync(staged, { recursive: true });
  writeFileSync(join(staged, basename(path)), process.env.HOLD_TEXT);
  const mode = statSync(staged).mode & 0o777;
  chmodSync(staged, mode & ~0o111);
  renameSync(staged, folder);
  release = () => chmodSync(folder, mode);
}
process.stdout.write("held\\n");
process.stdin.resume();
process.stdin.on("end", () => { release(); process.stdout.write("released\\n"); });
`;

/** How `HOLDER` ended once let go: its exit code and all it said. */
interface HolderEnd {
  readonly code: number | null;
  readonly said: string;
}

/**
 * Starts `HOLDER` writing `content` to `file` and returns once the file is written and nothing can read it, with what
 * lets it go: it ends the holder's input and waits for the holder to end, which gives the file back. A holder that
 * fails to start, or a file still readable, lets it go before the error is thrown.
 */
async function holdFromReads(file: string, content: string): Promise<() => Promise<HolderEnd>> {
  const child = spawn(process.execPath, ["--input-type=module", "-e", HOLDER], {
    env: environment({ HOLD_PATH: file, HOLD_TEXT: content, HOLD_STAGING: join(root, "staged") }), stdio: ["pipe", "pipe", "inherit"], windowsHide: true,
  });
  let said = "";
  child.stdout.setEncoding("utf8");
  const closed = once(child, "close");
  await new Promise<void>((held, failed) => {
    child.stdout.on("data", (chunk: string) => { said += chunk; if (said.includes("held\n")) held(); });
    void closed.then(([code]) => { failed(new Error(`the holder ended before it held ${file}, code ${String(code)}`)); }, failed);
  });
  const release = async (): Promise<HolderEnd> => {
    child.stdin.end();
    const [code] = await closed;
    return { code: code as number | null, said };
  };
  try {
    await expect(readFile(file)).rejects.toThrow();
  } catch (error) {
    await release();
    throw error;
  }
  return release;
}

/** The steer reached the coding agent in the hook's answer: it names the rule and the file. */
function expectSteered(answer: HookAnswer, file: string): void {
  expect(answer.stderr).toBe("");
  expect(contextOf(answer)).toContain(`[${RULE}]`);
  expect(contextOf(answer)).toContain(basename(file));
}

/** The steer reached the coding agent in one of a command's hook answers, and each hook answered without an error. */
function expectSteeredInOne(answers: readonly HookAnswer[], file: string): void {
  for (const answer of answers) expect(answer.stderr).toBe("");
  expect(answers.map(contextOf).filter(context => context.includes(`[${RULE}]`)).join("\n")).toContain(basename(file));
}

interface SupportBundle {
  session: {
    audits: Array<{ violations: Array<{ ruleId: string; path: string }>; failures: Array<{ path: string | null }> }>;
  };
}

let bundles = 0;

/** The session's record as `rerail support-bundle` writes it, each time to a file of its own. */
async function supportBundle(session: Session): Promise<SupportBundle> {
  bundles += 1;
  const bundleFile = join(root, `${session.id}-${bundles}.json`);
  await rerail(["support-bundle", "--session", session.id, "--output", bundleFile]);
  return JSON.parse(await readFile(bundleFile, "utf8")) as SupportBundle;
}

/** Waits until the session's record holds a failure to read `file`: a check of the session met the held file. */
async function expectReadFailed(session: Session, file: string): Promise<void> {
  await vi.waitFor(async () => {
    const failed = (await supportBundle(session)).session.audits.flatMap(audit => audit.failures).map(failure => basename(failure.path ?? ""));
    expect(failed).toContain(basename(file));
  }, { timeout: 60_000, interval: 250 });
}

/** `rerail session` counts the session's rules, and the support bundle logs the rule's violation in each file. */
async function expectRulesFired(session: Session, files: readonly string[]): Promise<void> {
  const status = await rerail(["session", "claude-code", session.id]);
  expect(status).not.toContain("rules unavailable");
  expect(status).toMatch(/\d+ rules? · steered \d+× in this session/u);
  const bundle = await supportBundle(session);
  const logged = bundle.session.audits.flatMap(audit => audit.violations).filter(violation => violation.ruleId === RULE).map(violation => basename(violation.path));
  expect(new Set(logged)).toEqual(new Set(files.map(file => basename(file))));
}

/**
 * An engine socket in front of the daemon's that holds every event `delayMs` before passing it on, with the daemon's
 * capability: a daemon that takes that long to answer each event, as one does while it walks a large checkout. It
 * passes a proof's challenge on at once, as the daemon answers a challenge before it reads any event. On Windows this
 * socket is a pipe of the test's own process, which Windows does not name as the daemon, so a hook sends its event
 * only once the engine behind it proves it holds the capability in daemon.json.
 */
async function slowEngine(delayMs: number): Promise<string> {
  const state = await daemonState();
  const path = process.platform === "win32" ? `\\\\.\\pipe\\rerail-slow-engine-${process.pid}-${Date.now()}` : join(root, "slow-engine.sock");
  const server = createServer(async (incoming, outgoing) => {
    const body = await text(incoming);
    const challenge = incoming.headers[ENGINE_CHALLENGE_HEADER];
    if (incoming.url !== ENGINE_PROOF_PATH) await delay(delayMs);
    const forwarded = request({
      socketPath: state.kernelSocketPath, method: "POST", path: incoming.url,
      headers: {
        "content-type": "application/json", "content-length": Buffer.byteLength(body), "x-rerail-access": state.accessToken ?? "",
        ...(challenge === undefined ? {} : { [ENGINE_CHALLENGE_HEADER]: challenge }),
      },
    }, answer => {
      outgoing.writeHead(answer.statusCode ?? 502, { "content-type": "application/json" });
      answer.pipe(outgoing);
    });
    forwarded.on("error", () => outgoing.destroy());
    forwarded.end(body);
  });
  servers.push(server);
  await new Promise<void>(done => server.listen(path, done));
  return path;
}

beforeAll(async () => {
  root = await realpath(await mkdtemp(join(tmpdir(), "rerail-claude-rules-")));
  repo = join(root, "app");
  worktree = join(root, "lanes", "lane");
  await mkdir(join(repo, "src", "app"), { recursive: true });
  await writeFile(join(repo, "src", "app", "__init__.py"), "");
  await writeFile(join(repo, "pyproject.toml"), "[project]\nname = \"app\"\nversion = \"0.1.0\"\n");
  await git(["init", "-q", "-b", "main"], repo);
  await git(["add", "."], repo);
  await git(["commit", "-q", "-m", "base"], repo);
  await git(["worktree", "add", "-q", "-b", "lane", worktree], repo);
  await rerail(["packs", "add", join(PACKAGE_ROOT, "packs", "complexity")], root);
  await rerail(["rules", "approve", PACK], root);
  setUpTestRepository(repo, { packs: ["complexity"] });
  await startDaemon();
}, 120_000);

afterAll(async () => {
  await Promise.all(servers.splice(0).map(server => new Promise(done => server.close(done))));
  await stopDaemon();
  await removeFolder(root);
}, 60_000);

describe("the Claude Code hook against a daemon of this build", () => {
  it("steers and logs a broken rule in the session's checkout, in a subagent's edit and in a linked worktree", async () => {
    const session: Session = { id: "rules-fire-session", projectDir: repo };
    const files = [join(repo, "src", "app", "main_edit.py"), join(repo, "src", "app", "subagent_edit.py"), join(worktree, "src", "app", "worktree_edit.py")];
    await startSession(session);
    expectSteered(await writeTooComplex(session, files[0]!), files[0]!);
    expectSteered(await writeTooComplex(session, files[1]!, "agent-1"), files[1]!);
    expectSteered(await writeTooComplex(session, files[2]!), files[2]!);
    await endTurn(session);
    await expectRulesFired(session, files);
  }, 120_000);

  // A folder with no search permission keeps no file from root, which reads it all the same.
  it.skipIf(process.platform !== "win32" && process.getuid?.() === 0)("steers and logs a Bash command's write at the next command when another program held the file through the check at its result", async () => {
    const session: Session = { id: "held-write-session", projectDir: repo };
    const file = join(repo, "src", "app", "held", "held_write.py");
    await startSession(session);
    let release = async (): Promise<HolderEnd | undefined> => undefined;
    let ended: HolderEnd | undefined;
    try {
      await runCommand(session, "toolu_held_write", async () => { release = await holdFromReads(file, tooComplex("held_write")); });
      // The state the steer below is about: the session's checks met the file only held, and recorded the failed read.
      await expectReadFailed(session, file);
    } finally {
      ended = await release();
    }
    // Checked after the holder let go, so a failure above is the one reported.
    expect(ended).toEqual({ code: 0, said: expect.stringContaining("released\n") });
    expectSteeredInOne(await runCommand(session, "toolu_after_held_write", async () => undefined), file);
    await endTurn(session);
    await expectRulesFired(session, [file]);
  }, 120_000);

  it.runIf(process.platform === "win32")("steers and logs it in a session opened from VS Code, which spells the drive letter in lower case", async () => {
    const projectDir = repo.replace(/^[A-Z]:/u, drive => drive.toLowerCase());
    const session: Session = { id: "vscode-spelling-session", projectDir };
    const files = [join(projectDir, "src", "app", "vscode_edit.py"), join(projectDir, "src", "app", "vscode_subagent_edit.py")];
    await startSession(session);
    expectSteered(await writeTooComplex(session, files[0]!), files[0]!);
    expectSteered(await writeTooComplex(session, files[1]!, "agent-1"), files[1]!);
    await endTurn(session);
    await expectRulesFired(session, files);
  }, 120_000);

  it("steers and logs it in every edit of subagents whose hooks run at once while the daemon takes seconds per event", async () => {
    // Eight subagents' hooks, each event held 4 s: hooks that waited on each other's whole round trip passed 30 s.
    const session: Session = { id: "parallel-subagents-session", projectDir: repo, env: { RERAIL_ENGINE_SOCKET: await slowEngine(4_000) } };
    const files = Array.from({ length: 8 }, (_, agent) => join(repo, "src", "app", `parallel_edit_${agent}.py`));
    await startSession(session);
    const answers = await Promise.all(files.map((file, agent) => writeTooComplex(session, file, `agent-${agent}`)));
    answers.forEach((answer, index) => expectSteered(answer, files[index]!));
    await endTurn(session);
    await expectRulesFired(session, files);
  }, 180_000);

  // #535: the session that wrote it from the shell, its command running in the worktree, was never steered on it.
  // #534: every other session open recorded it in its own history.
  it("steers a shell write in a linked worktree on the session whose command ran there, and records one aimed there from elsewhere once, in no session", async () => {
    const writer: Session = { id: "worktree-shell-writer", projectDir: repo };
    const reader: Session = { id: "worktree-shell-reader", projectDir: repo };
    await startSession(writer);
    await startSession(reader);
    const ranThere = join(worktree, "src", "app", "shell_there.py");
    const aimedThere = join(worktree, "src", "app", "shell_aimed.py");
    let call = 0;
    /** A Bash call of `session` whose command runs in `cwd` and writes `file`, if any; what its two answers told the agent. */
    const bash = async (session: Session, cwd: string, file?: string): Promise<string> => {
      const common = { session_id: session.id, cwd, tool_name: "Bash", tool_use_id: `toolu_bash_${call += 1}`, tool_input: { command: "./write.sh" } };
      const pre = await hook({ ...common, hook_event_name: "PreToolUse" }, session.projectDir);
      if (file !== undefined) await writeFile(file, tooComplex(basename(file, ".py")));
      const post = await hook({ ...common, hook_event_name: "PostToolUse", tool_response: { stdout: "", stderr: "" } }, session.projectDir);
      return `${contextOf(pre)}${contextOf(post)}`;
    };
    // The walk of the other worktrees runs behind the command, so the steer comes with a later answer of the session.
    const steerOf = async (session: Session, cwd: string, first: string): Promise<string> => {
      let told = first;
      for (let tries = 0; !told.includes(`[${RULE}]`) && tries < 20; tries += 1) {
        await delay(500);
        told = await bash(session, cwd);
      }
      return told;
    };

    expect(await steerOf(writer, worktree, await bash(writer, worktree, ranThere))).toContain(basename(ranThere));
    expect(await steerOf(writer, repo, await bash(writer, repo, aimedThere))).not.toContain(basename(aimedThere));
    await bash(reader, repo);
    await endTurn(writer);
    await endTurn(reader);
    // Once the walks have settled: the writer's history holds what it wrote there, the reader's nothing of either, and
    // the write aimed there from the main checkout is the repository's, recorded once.
    await delay(6_000);
    const database = new Database(join(process.env.RERAIL_HOME!, "rerail.sqlite"), { readonly: true });
    try {
      const recorded = (file: string) => database.prepare(`SELECT audit.session_id AS session, audit.mode AS mode FROM audits audit, json_each(audit.violations_json) violation
        WHERE json_extract(violation.value, '$.ruleId') = ? AND json_extract(violation.value, '$.path') LIKE ?`).all(RULE, `%${basename(file)}`);
      expect(recorded(ranThere)).toEqual([expect.objectContaining({ session: writer.id, mode: "steer" })]);
      expect(recorded(aimedThere)).toEqual([{ session: null, mode: "log" }]);
    } finally {
      database.close();
    }
  }, 180_000);

  // The walk of the other worktrees runs behind the turn's end, which does not wait for it: the last write of a turn
  // there is told at the session's next answer, the next turn's first tool call.
  it("steers a shell write in a linked worktree made just before the turn ends, at the session's next tool call", async () => {
    const writer: Session = { id: "worktree-last-write", projectDir: repo };
    await startSession(writer);
    const file = join(worktree, "src", "app", "last_write.py");
    const write = { session_id: writer.id, cwd: worktree, tool_name: "Bash", tool_use_id: "toolu_last_1", tool_input: { command: "./write.sh" } };
    const told: string[] = [];
    told.push(contextOf(await hook({ ...write, hook_event_name: "PreToolUse" }, repo)));
    await writeFile(file, tooComplex("last_write"));
    told.push(contextOf(await hook({ ...write, hook_event_name: "PostToolUse", tool_response: { stdout: "", stderr: "" } }, repo)));
    told.push(contextOf(await endTurn(writer)));
    await delay(6_000);
    await hook({ session_id: writer.id, cwd: worktree, hook_event_name: "UserPromptSubmit", prompt_id: "prompt-2", prompt: "Next." }, repo);
    const next = { session_id: writer.id, cwd: worktree, tool_name: "Bash", tool_use_id: "toolu_last_2", tool_input: { command: "ls" } };
    told.push(contextOf(await hook({ ...next, hook_event_name: "PreToolUse" }, repo)));
    expect(told.join("\n")).toContain(`last_write.py:1:1`);
    expect(told.join("\n")).toContain(`[${RULE}]`);
  }, 120_000);

  it("steers and logs it after the daemon restarts mid-session", async () => {
    const session: Session = { id: "daemon-restart-session", projectDir: repo };
    const files = [join(repo, "src", "app", "before_restart.py"), join(repo, "src", "app", "after_restart.py")];
    await startSession(session);
    expectSteered(await writeTooComplex(session, files[0]!), files[0]!);
    await stopDaemon();
    await startDaemon();
    // The edit's own PreToolUse reopens the session on the new daemon. Its call waits for the session's rules to open
    // (`REOPENED_RULES_WAIT_MS`), so the edit is checked and steered at its own PostToolUse, as before the restart.
    expectSteered(await writeTooComplex(session, files[1]!, "agent-1"), files[1]!);
    await expectRulesFired(session, files);
  }, 120_000);
});
