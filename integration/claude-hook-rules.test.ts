/**
 * Rules fire in a Claude Code session: the built hook, run as Claude Code runs it (`dist/harness/hook-launcher.js`
 * starting `dist/hosts/claude/claude-hook.js`, one process per event, with the session's `CLAUDE_PROJECT_DIR`), replays
 * a session's events against a daemon of this build in this file's own Rerail home. The repository runs the complexity
 * pack from this checkout, approved, so a Python function above the cyclomatic ceiling breaks
 * `complexity-cyclomatic-python`. Each session edits such a function in the session's checkout, through a subagent
 * (whose tool calls Claude Code sends under the parent's session id) and in a linked worktree outside the checkout,
 * and the test reads back what a person reads: the steer in the hook's answer, the session's rule count from
 * `rerail session`, and the logged violation from `rerail support-bundle`.
 *
 * Two cases go red on causes that left a person's sessions with "rules unavailable" or edits unchecked: a session opened
 * from VS Code, which spells the drive letter `c:` (Windows), and subagents whose hooks run at once while the daemon is
 * slow to answer. The last case checks that a session's edits are still checked after its daemon restarts.
 */
import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { createServer, request, type Server } from "node:http";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { setTimeout as delay } from "node:timers/promises";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { ENTRY_POINTS } from "../src/engine/index.js";
import { DAEMON_STATE_FILE, type DaemonState } from "../src/runtime/index.js";
import { setUpTestRepository } from "../test/support/repository-rules.js";

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
}

async function endTurn(session: Session): Promise<void> {
  const common = { session_id: session.id, cwd: session.projectDir };
  await hook({ ...common, hook_event_name: "Stop", prompt_id: "prompt-1", last_assistant_message: "Done." }, session.projectDir, session.env);
}

/** The steer reached the coding agent in the hook's answer: it names the rule and the file. */
function expectSteered(answer: HookAnswer, file: string): void {
  expect(answer.stderr).toBe("");
  expect(contextOf(answer)).toContain(`[${RULE}]`);
  expect(contextOf(answer)).toContain(basename(file));
}

/** `rerail session` counts the session's rules, and the support bundle logs the rule's violation in each file. */
async function expectRulesFired(session: Session, files: readonly string[]): Promise<void> {
  const status = await rerail(["session", "claude-code", session.id]);
  expect(status).not.toContain("rules unavailable");
  expect(status).toMatch(/\d+ rules? · steered \d+× in this session/u);
  const bundleFile = join(root, `${session.id}.json`);
  await rerail(["support-bundle", "--session", session.id, "--output", bundleFile]);
  const bundle = JSON.parse(await readFile(bundleFile, "utf8")) as { session: { audits: Array<{ violations: Array<{ ruleId: string; path: string }> }> } };
  const logged = bundle.session.audits.flatMap(audit => audit.violations).filter(violation => violation.ruleId === RULE).map(violation => basename(violation.path));
  expect(new Set(logged)).toEqual(new Set(files.map(file => basename(file))));
}

/**
 * An engine socket in front of the daemon's that holds every event `delayMs` before passing it on, with the daemon's
 * capability: a daemon that takes that long to answer each event, as one does while it walks a large checkout.
 */
async function slowEngine(delayMs: number): Promise<string> {
  const state = await daemonState();
  const path = process.platform === "win32" ? `\\\\.\\pipe\\rerail-slow-engine-${process.pid}-${Date.now()}` : join(root, "slow-engine.sock");
  const server = createServer(async (incoming, outgoing) => {
    const body = await text(incoming);
    await delay(delayMs);
    const forwarded = request({
      socketPath: state.kernelSocketPath, method: "POST", path: incoming.url,
      headers: { "content-type": "application/json", "content-length": Buffer.byteLength(body), "x-rerail-access": state.accessToken ?? "" },
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
  await rm(root, { recursive: true, force: true, maxRetries: 10 });
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

  it("steers and logs it after the daemon restarts mid-session", async () => {
    const session: Session = { id: "daemon-restart-session", projectDir: repo };
    const files = [join(repo, "src", "app", "before_restart.py"), join(repo, "src", "app", "after_restart.py")];
    await startSession(session);
    expectSteered(await writeTooComplex(session, files[0]!), files[0]!);
    await stopDaemon();
    await startDaemon();
    expectSteered(await writeTooComplex(session, files[1]!, "agent-1"), files[1]!);
    await endTurn(session);
    await expectRulesFired(session, files);
  }, 120_000);
});
