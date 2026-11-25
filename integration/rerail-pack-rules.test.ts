/**
 * Rules written in `.rerail` fire in a Claude Code session and in `rerail scan`: the built hook replays a session's
 * events against a daemon of this build in this file's own Rerail home, as `claude-hook-rules.test.ts` does. The
 * repository runs the fixture pack `rr-sample` from this checkout, approved, with `rr-sample-print-call` set to Log and
 * `rr-sample-assert` turned off in `app/off/`, beside the complexity pack's JSON rules.
 *
 * A `.rerail` run goes on after the edit, on the files on disk, once the checkout is quiet or the turn ends: the test
 * ends the turn and reads the steer at the boundaries that follow, and the run's audit from the Rerail database.
 */
import { execFile, spawn, type ChildProcess } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve, win32 } from "node:path";
import { text } from "node:stream/consumers";
import { promisify } from "node:util";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import {
  type AuditRecord,
  defaultRerailDatabasePath,
  ENTRY_POINTS,
  openRerailDatabase,
  setRuleSetting,
  SqliteAuditReader,
} from "../src/engine/index.js";
import { DAEMON_STATE_FILE } from "../src/runtime/index.js";
import { fixturesPath } from "../test/support/oracles.js";
import { removeFolder } from "../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../test/support/repository-rules.js";
import { awaitCodeRulesOpenThrough } from "./support/code-rules-open.js";

const PACKAGE_ROOT = resolve(".");
const CLI = join(PACKAGE_ROOT, "dist", "cli", "cli.js");
const HOOK = [join(PACKAGE_ROOT, "dist", "harness", "hook-launcher.js"), process.execPath, join(PACKAGE_ROOT, "dist", "hosts", "claude", "claude-hook.js")];
const JSON_RULE = "complexity-cyclomatic-python";

let root: string;
let repo: string;
let daemon: ChildProcess | undefined;

const environment = (extra: NodeJS.ProcessEnv = {}): NodeJS.ProcessEnv => ({ ...process.env, BROWSER: "none", ...extra });

async function rerail(args: readonly string[], extra: NodeJS.ProcessEnv = {}): Promise<string> {
  const { stdout } = await promisify(execFile)(process.execPath, [CLI, ...args], { cwd: repo, env: environment(extra), windowsHide: true });
  return stdout;
}

async function git(args: readonly string[]): Promise<void> {
  await promisify(execFile)("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd: repo, windowsHide: true });
}

/**
 * A daemon of this build in the file's Rerail home, with `extra` in its environment, answering once its state file is
 * written. Its quiet period is longer than any test, so each `.rerail` run starts at a turn's end with all the turn's
 * writes.
 */
async function startDaemon(extra: NodeJS.ProcessEnv = {}): Promise<void> {
  const home = process.env.RERAIL_HOME!;
  const child = spawn(process.execPath, [join(PACKAGE_ROOT, ENTRY_POINTS.daemon), "--port", "0", "--state-file", join(home, DAEMON_STATE_FILE)], {
    env: environment({ RERAIL_RR_QUIET_MS: "3600000", ...extra }), stdio: ["ignore", "pipe", "ignore"], windowsHide: true,
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
  output: { hookSpecificOutput?: { additionalContext?: string }; systemMessage?: string };
  stderr: string;
}

async function hook(input: Record<string, unknown>): Promise<HookAnswer> {
  const child = spawn(process.execPath, HOOK, {
    cwd: repo, env: environment({ CLAUDE_PROJECT_DIR: repo }), stdio: ["pipe", "pipe", "pipe"], windowsHide: true,
  });
  child.stdin.end(JSON.stringify(input));
  const [stdout, stderr] = await Promise.all([text(child.stdout), text(child.stderr), once(child, "close")]);
  return { output: JSON.parse(stdout) as HookAnswer["output"], stderr };
}

const contextOf = (answer: HookAnswer): string => answer.output.hookSpecificOutput?.additionalContext ?? "";

class Session {
  readonly id: string;
  #prompt = 0;
  #tool = 0;

  constructor(id: string) {
    this.id = id;
  }

  async start(): Promise<void> {
    await hook({ session_id: this.id, cwd: repo, hook_event_name: "SessionStart", source: "startup" });
    await this.prompt();
  }

  async prompt(): Promise<void> {
    this.#prompt += 1;
    await hook({ session_id: this.id, cwd: repo, hook_event_name: "UserPromptSubmit", prompt_id: `prompt-${this.#prompt}`, prompt: "Go on." });
  }

  /** A Write of `content` to `path` (relative to the repository): PreToolUse, the write, PostToolUse; the PostToolUse answer. */
  async write(path: string, content: string): Promise<HookAnswer> {
    const file = join(repo, path);
    const tool = { session_id: this.id, cwd: repo, tool_name: "Write", tool_use_id: `toolu_${this.id}_${(this.#tool += 1)}`, tool_input: { file_path: file, content } };
    expect((await hook({ ...tool, hook_event_name: "PreToolUse" })).stderr).toBe("");
    await mkdir(dirname(file), { recursive: true });
    await writeFile(file, content);
    return await hook({ ...tool, hook_event_name: "PostToolUse", tool_response: { type: "create", filePath: file, content } });
  }

  /** A Read, whose PreToolUse is the session's next boundary: its answer. */
  async read(path: string): Promise<HookAnswer> {
    const tool = { session_id: this.id, cwd: repo, tool_name: "Read", tool_use_id: `toolu_${this.id}_${(this.#tool += 1)}`, tool_input: { file_path: join(repo, path) } };
    return await hook({ ...tool, hook_event_name: "PreToolUse" });
  }

  /**
   * Waits for the session's code rules, through Read calls that edit nothing. A session's first start answers its tool
   * calls without code rules while they open (the first index of the checkout, its rules compiled), and the first pass
   * after checks what was edited meanwhile; only an edit made once they run is steered at its own result.
   */
  async awaitCodeRules(): Promise<void> {
    await awaitCodeRulesOpenThrough(this.id, async () => {
      const tool = { session_id: this.id, cwd: repo, tool_name: "Read", tool_use_id: `toolu_${this.id}_${(this.#tool += 1)}`, tool_input: { file_path: join(repo, "app", "legacy.py") } };
      await hook({ ...tool, hook_event_name: "PreToolUse" });
      await hook({ ...tool, hook_event_name: "PostToolUse", tool_response: { type: "text" } });
    });
  }

  /** The turn's end: `Stop`'s answer. */
  async endTurn(): Promise<HookAnswer> {
    return await hook({ session_id: this.id, cwd: repo, hook_event_name: "Stop", prompt_id: `prompt-${this.#prompt}`, last_assistant_message: "Done." });
  }
}

/** Every audit recorded, newest first. */
function audits(): AuditRecord[] {
  return new SqliteAuditReader(defaultRerailDatabasePath()).list({ limit: 200 }).audits;
}

/** Waits for the finished audit of a `.rerail` run (one that carried only `.rerail` rules) over `path`. */
async function rerailAuditOf(path: string): Promise<AuditRecord> {
  let found: AuditRecord | undefined;
  await vi.waitFor(() => {
    found = audits().find(audit => audit.paths.includes(path) && audit.status !== "running"
      && audit.ruleProvenance.length > 0 && audit.ruleProvenance.every(rule => rule.ruleId.startsWith("rr-sample-")));
    expect(found).toBeDefined();
  }, { timeout: 120_000, interval: 250 });
  return found!;
}

/** The `.rerail` runner's children of the daemon still running, by their command lines. */
async function runnerChildren(): Promise<string[]> {
  const parent = daemon!.pid!;
  return (process.platform === "win32" ? await windowsChildren(parent) : await posixChildren(parent)).filter(line => line.includes("rr-child"));
}

async function posixChildren(parent: number): Promise<string[]> {
  const { stdout } = await promisify(execFile)("ps", ["-A", "-o", "ppid=,command="]);
  return stdout.split("\n").filter(line => line.trim().startsWith(`${parent} `));
}

/**
 * Windows has no `ps` of its own, and Git Bash's answers `unknown option -- A`: Windows PowerShell reads the process
 * table through CIM there, its cmdlet named with its module, so PowerShell loads that one module by its name and reads
 * no other installed module first.
 */
async function windowsChildren(parent: number): Promise<string[]> {
  const powershell = win32.join(process.env.SystemRoot ?? String.raw`C:\Windows`, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const { stdout } = await promisify(execFile)(powershell, [
    "-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
    `(CimCmdlets\\Get-CimInstance -ClassName Win32_Process -Filter 'ParentProcessId=${parent}').CommandLine`,
  ], { windowsHide: true });
  return stdout.split(/\r?\n/u).filter(line => line.trim() !== "");
}

const VIEWS = "def view(user):\n    assert user.is_admin\n    return user\n";
const REPORT = "from app.legacy import old_total\n\n\ndef report(items):\n    return old_total(items)\n";
const MAIN = "print(\"starting\")\n";
const OFF = "assert True\n";
/** A Python function at cyclomatic complexity 10, over the complexity pack's ceiling, with an assert in it. */
const TOO_COMPLEX = `def classify(a, b, c, d, e, f, g):\n    score = 0\n${["a", "b", "c", "d", "e", "f", "g"].map((flag, index) => `    if ${flag}:\n        score += ${index + 1}\n`).join("")}    assert score >= 0\n    if a and b:\n        score += 8\n    return score\n`;

beforeAll(async () => {
  root = await realpath(await mkdtemp(join(tmpdir(), "rerail-rr-rules-")));
  repo = join(root, "app-repo");
  await mkdir(join(repo, "app"), { recursive: true });
  await writeFile(join(repo, "app", "__init__.py"), "");
  await writeFile(join(repo, "app", "legacy.py"), "@deprecated\ndef old_total(items):\n    return sum(items)\n");
  await git(["init", "-q", "-b", "main"]);
  await git(["add", "."]);
  await git(["commit", "-q", "-m", "base"]);
  await rerail(["packs", "add", fixturesPath("packs", "rr-sample")]);
  await rerail(["rules", "approve", "@neralabs/rerail-pack-rr-sample"]);
  await rerail(["packs", "add", join(PACKAGE_ROOT, "packs", "complexity")]);
  await rerail(["rules", "approve", "@neralabs/rerail-pack-complexity"]);
  const repositoryKey = setUpTestRepository(repo, { packs: ["rr-sample", "complexity"] });
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    const owner = { kind: "repository" as const, repositoryKey };
    setRuleSetting(database, { owner, folder: "", targetKind: "rule", targetId: "rr-sample-print-call", on: null, mode: "log" });
    setRuleSetting(database, { owner, folder: "app/off", targetKind: "rule", targetId: "rr-sample-assert", on: false, mode: null });
  } finally {
    database.$client.close();
  }
  await startDaemon();
}, 180_000);

afterAll(async () => {
  await stopDaemon();
  await removeFolder(root);
}, 60_000);

describe("rules written in .rerail, against a daemon of this build", () => {
  it("steers a one-file rule and a rule that reads another file at the next boundary, records a Log rule's finding without steering it, and keeps each span", async () => {
    const session = new Session("rr-rules-session");
    await session.start();
    for (const [path, content] of [["app/views.py", VIEWS], ["app/report.py", REPORT], ["app/main.py", MAIN]] as const)
      expect((await session.write(path, content)).stderr).toBe("");
    const stop = await session.endTurn();
    const audit = await rerailAuditOf("app/views.py");
    expect(audit.status).toBe("completed");
    expect(audit.violations.map(({ ruleId, path, mode, span }) => ({ ruleId, path, mode, line: span?.startLine }))).toEqual(expect.arrayContaining([
      { ruleId: "rr-sample-assert", path: "app/views.py", mode: "steer", line: 1 },
      { ruleId: "rr-sample-deprecated-call", path: "app/report.py", mode: "steer", line: 4 },
      { ruleId: "rr-sample-print-call", path: "app/main.py", mode: "log", line: 0 },
    ]));
    await session.prompt();
    const answers = [stop, await session.read("app/views.py")];
    const steered = answers.map(contextOf).join("\n");
    expect(steered).toContain("[rr-sample-assert]");
    expect(steered).toContain("[rr-sample-deprecated-call]");
    expect(steered).toContain("report.py");
    expect(steered).not.toContain("[rr-sample-print-call]");
    // The person's line names each rule by the title its pack.json gives it.
    const lines = answers.map(answer => answer.output.systemMessage ?? "").join("\n");
    expect(lines).toContain("No assert statements");
    expect(lines).toContain("No calls to deprecated functions");
  }, 240_000);

  it("does not run a rule turned off where the file was written", async () => {
    const session = new Session("rr-rule-off-session");
    await session.start();
    await session.write("app/off/check.py", OFF);
    await session.endTurn();
    const audit = await rerailAuditOf("app/off/check.py");
    expect(audit.ruleProvenance.map(rule => rule.ruleId)).not.toContain("rr-sample-assert");
    expect(audit.violations).toEqual([]);
  }, 240_000);

  it("records a run killed past its deadline as its rules' failure, while the JSON rule of the same edit steers on time and no runner is left", async () => {
    await stopDaemon();
    await startDaemon({ RERAIL_RR_DEADLINE_MS: "1" });
    try {
      const session = new Session("rr-deadline-session");
      await session.start();
      // The JSON rule's steer is read at the edit's own result, so the edit waits for the session's code rules.
      await session.awaitCodeRules();
      const answer = await session.write("app/slow.py", TOO_COMPLEX);
      expect(contextOf(answer)).toContain(`[${JSON_RULE}]`);
      await session.endTurn();
      const audit = await rerailAuditOf("app/slow.py");
      expect(audit.status).toBe("failed");
      expect(audit.failures).toEqual(expect.arrayContaining([
        expect.objectContaining({ ruleId: "rr-sample-assert", code: "budget-exceeded", message: expect.stringContaining("RERAIL_RR_DEADLINE_MS") }),
      ]));
      expect(audit.violations).toEqual([]);
      expect(await runnerChildren()).toEqual([]);
    } finally {
      await stopDaemon();
      await startDaemon();
    }
  }, 240_000);

  it("reports the .rerail rules' findings in rerail scan, in the same audit as the JSON rules'", async () => {
    const report = JSON.parse(await rerail(["scan", "--repo", repo, "--json", "--no-send"], { RERAIL_EXPERIMENTAL_HEADLESS_SCAN: "1" })) as {
      rules: { ruleId: string }[]; violations: { ruleId: string; path: string; start: { line: number } | null }[];
    };
    expect(report.rules.map(rule => rule.ruleId)).toEqual(expect.arrayContaining(["rr-sample-assert", "rr-sample-deprecated-call", JSON_RULE]));
    expect(report.violations.map(({ ruleId, path, start }) => ({ ruleId, path, line: start?.line }))).toEqual(expect.arrayContaining([
      { ruleId: "rr-sample-assert", path: "app/views.py", line: 2 },
      { ruleId: "rr-sample-deprecated-call", path: "app/report.py", line: 5 },
      { ruleId: JSON_RULE, path: "app/slow.py", line: 1 },
    ]));
    expect(report.violations.map(violation => violation.path)).not.toContain("app/off/check.py");
  }, 240_000);
});
