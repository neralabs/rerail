/**
 * An older Node at the path set-up recorded, as a package downgrade or Homebrew's `<prefix>/bin/node` linked to an
 * older formula leaves it: the launcher and the hooks run that path while it is there, whatever PATH holds. The hook
 * then says in one line why Rerail is off and what to run; done as written in a person's shell (`sh` here, cmd.exe and
 * PowerShell on Windows) with this Node first on PATH, that step gets Rerail back: the hook gives its start note again.
 *
 * Set-up records a copy of this Node, which later reports itself as Node 22 (`OLDER_NODE_VERSION`, in
 * `process.versions.node`, which Rerail reads): every node of the run loads a module that changes only that copy, so
 * the Node first on PATH stays this one. Claude Code's hook runs as Claude Code runs it (through `sh`, or on Windows
 * through cmd.exe and its `if exist`), Codex's through `sh` or PowerShell and its `Test-Path`.
 */
import { spawn } from "node:child_process";
import { copyFile, link, mkdir, mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { once } from "node:events";
import { tmpdir } from "node:os";
import { delimiter, dirname, join, resolve } from "node:path";
import type { Writable } from "node:stream";
import { text } from "node:stream/consumers";
import { pathToFileURL } from "node:url";
import { afterAll, describe, expect, it } from "vitest";
import { ENTRY_POINTS } from "../../src/engine/index.js";
import { OLDER_NODE_VERSION } from "../../test/support/platform/older-node.js";
import { removeFolder } from "../../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../../test/support/repository-rules.js";

const WINDOWS = process.platform === "win32";
const PACKAGE_ROOT = resolve(".");
const NODE = WINDOWS ? "node.exe" : "node";
const folders: string[] = [];

afterAll(async () => {
  await Promise.all(folders.splice(0).map(folder => removeFolder(folder)));
});

interface Ran {
  code: number | null;
  stdout: string;
  stderr: string;
}

/**
 * `command` run to its end. Given no `input` it gets no stdin to read (the null device), so nothing is written to a
 * command that takes none, as git here, which can exit before a write reaches it. Given `input`, it may still end
 * without reading it, as a hook line that starts no node does. With `verbatim`, Windows gets `args` joined as written.
 */
async function run(command: string, args: readonly string[], cwd: string, env: NodeJS.ProcessEnv, input?: string, verbatim = false): Promise<Ran> {
  const options = { cwd, env, windowsHide: true, windowsVerbatimArguments: verbatim };
  const child = input === undefined
    ? spawn(command, [...args], { ...options, stdio: ["ignore", "pipe", "pipe"] })
    : spawn(command, [...args], { ...options, stdio: ["pipe", "pipe", "pipe"] });
  const [stdout, stderr, [code]] = await Promise.all([
    text(child.stdout), text(child.stderr), once(child, "close") as Promise<[number | null]>, fed(child.stdin, input),
  ]);
  return { code, stdout, stderr };
}

/**
 * Writes `input` to a child's stdin and closes it. A write to a child that has closed its stdin fails with EPIPE: the
 * child ended without reading, which is no failure, while any other error on the stream fails the run.
 */
async function fed(stdin: Writable | null, input: string | undefined): Promise<void> {
  if (stdin === null) return;
  const closed = Promise.withResolvers<void>();
  stdin.on("error", (error: NodeJS.ErrnoException) => {
    if (error.code === "EPIPE") closed.resolve();
    else closed.reject(error);
  });
  stdin.on("close", () => { closed.resolve(); });
  stdin.end(input);
  await closed.promise;
}

interface Place {
  root: string;
  user: string;
  repo: string;
  /** The node set-up records: a copy of this one, in a folder of its own, first on PATH. */
  recorded: string;
  env: NodeJS.ProcessEnv;
}

async function place(): Promise<Place> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-node-replaced-")));
  folders.push(root);
  // A home whose folder name holds a space: the step's path is quoted in every shell.
  const [user, repo] = [join(root, "John Smith"), join(root, "app")];
  const home = join(user, ".rerail");
  await Promise.all([home, repo, join(root, "system")].map(folder => mkdir(folder, { recursive: true })));
  const recorded = join(root, "system", NODE);
  await link(process.execPath, recorded).catch(async () => await copyFile(process.execPath, recorded));
  const env: NodeJS.ProcessEnv = {
    ...process.env, HOME: user, USERPROFILE: user, RERAIL_HOME: home,
    // The recorded node first, as `/usr/local/bin/node` is, and the launcher's folder after it.
    PATH: [dirname(recorded), join(home, "bin"), process.env.PATH ?? ""].join(delimiter),
  };
  delete env.CLAUDE_PROJECT_DIR;
  delete env.NODE_OPTIONS;
  await writeFile(join(repo, "README.md"), "app\n");
  for (const args of [["init", "-q"], ["add", "."], ["-c", "user.name=R", "-c", "user.email=r@example.invalid", "commit", "-q", "-m", "seed"]]) {
    const git = await run("git", args, repo, env);
    expect(git.code, git.stderr).toBe(0);
  }
  const workerHome = process.env.RERAIL_HOME;
  process.env.RERAIL_HOME = home;
  try {
    setUpTestRepository(repo, {}, join(home, "rerail.sqlite"));
  } finally {
    if (workerHome === undefined) delete process.env.RERAIL_HOME;
    else process.env.RERAIL_HOME = workerHome;
  }
  return { root, user, repo, recorded, env };
}

/** The environment once an older Node is at the recorded path: every node loads a module that makes only that one older. */
async function olderAtRecorded(p: Place): Promise<NodeJS.ProcessEnv> {
  const module = join(p.root, "older-at-recorded.mjs");
  const same = WINDOWS ? "(a, b) => a.toLowerCase() === b.toLowerCase()" : "(a, b) => a === b";
  await writeFile(module, [
    'import { realpathSync } from "node:fs";',
    `const same = ${same};`,
    `if (same(realpathSync.native(process.execPath), realpathSync.native(${JSON.stringify(p.recorded)}))) {`,
    `  Object.defineProperty(process.versions, "node", { value: ${JSON.stringify(OLDER_NODE_VERSION)} });`,
    "}",
    "",
  ].join("\n"));
  return { ...p.env, NODE_OPTIONS: `--import=${pathToFileURL(module).href}` };
}

interface Agent {
  id: "claude-code" | "codex";
  settings: string[];
}

const AGENTS: Agent[] = [{ id: "claude-code", settings: [".claude", "settings.json"] }, { id: "codex", settings: [".codex", "hooks.json"] }];

/**
 * The agent's SessionStart hook, run as the agent runs it, with the event on stdin: its start note, its answer and its
 * stderr. With `RERAIL_AUTOSTART=0`, as every test runs, the note is the line that says no daemon runs.
 */
async function sessionStart(p: Place, agent: Agent, env: NodeJS.ProcessEnv): Promise<{ note: string; stdout: string; stderr: string }> {
  const settings = JSON.parse(await readFile(join(p.user, ...agent.settings), "utf8")) as { hooks: Record<string, Array<{ hooks: Array<{ command: string; args?: string[] }> }>> };
  const entry = settings.hooks.SessionStart?.[0]?.hooks[0];
  if (entry === undefined) throw new Error(`no SessionStart hook in ${agent.settings.join("/")}`);
  const [command, args] = entry.args !== undefined
    ? [entry.command, entry.args]
    : WINDOWS ? ["powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", entry.command]] : ["/bin/sh", ["-c", entry.command]];
  const event = JSON.stringify({ session_id: `${agent.id}-session`, cwd: p.repo, hook_event_name: "SessionStart", source: "startup" });
  const ran = await run(command, args, p.repo, { ...env, CLAUDE_PROJECT_DIR: p.repo }, event);
  const answer = JSON.parse(ran.stdout.trim() || "{}") as { hookSpecificOutput?: { additionalContext?: string }; systemMessage?: string };
  return { note: answer.hookSpecificOutput?.additionalContext ?? answer.systemMessage ?? "", stdout: ran.stdout.trim(), stderr: ran.stderr };
}

/**
 * The step the hook's line on stderr names, as a person copies it: on that line alone, the words after "then run", to
 * the sentence on what the rerail command runs or to the line's end.
 */
function namedStep(stderr: string): string {
  const line = stderr.split(/\r?\n/u).find(each => each.startsWith("rerail hook: Rerail needs Node "));
  const step = /, then run (.+?)(?:: the rerail command runs .+)?\.$/u.exec(line ?? "")?.[1];
  if (step === undefined) throw new Error(`the hook named no step: ${stderr}`);
  return step;
}

/** The shells a person types the step in: `sh` on macOS and Linux; on Windows cmd.exe and PowerShell, which both take its double quotes. */
const SHELLS: ReadonlyArray<"sh" | "cmd.exe" | "PowerShell"> = WINDOWS ? ["cmd.exe", "PowerShell"] : ["sh"];

/** `step` typed into `shell`. cmd.exe gets it as written, `/s /c "<step>"`: it reads quotes by its own rules, not the C runtime's. */
async function inShell(shell: (typeof SHELLS)[number], step: string, cwd: string, env: NodeJS.ProcessEnv): Promise<Ran> {
  if (shell === "cmd.exe") return await run("cmd.exe", ["/d", "/s", "/c", `"${step}"`], cwd, env, undefined, true);
  return shell === "PowerShell"
    ? await run("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", step], cwd, env)
    : await run("/bin/sh", ["-c", step], cwd, env);
}

describe("an older Node at the path set-up recorded", () => {
  it("reads the step from the hook's own line, whatever other line stderr holds", () => {
    const line = "rerail hook: Rerail needs Node 24 or newer and this is Node 22.20.0, so it is off in this session. Put Node 24 or newer first on PATH, then run rerail install --refresh.";
    expect(namedStep(`${line}\n(node:1) Warning: a second line\n`)).toBe("rerail install --refresh");
  });

  it.each(AGENTS.flatMap(agent => SHELLS.map(shell => ({ ...agent, shell }))))("leaves $id's hook naming a step that, done as written in $shell with Node 24 first on PATH, gets its start note back", async ({ shell, ...agent }) => {
    const p = await place();
    const installed = await run(p.recorded, [join(PACKAGE_ROOT, ENTRY_POINTS.cli), "install", "--root", PACKAGE_ROOT, "--agents", agent.id], p.root, p.env);
    expect(installed.code, installed.stderr).toBe(0);
    const atInstall = await sessionStart(p, agent, p.env);
    expect(atInstall.note, `${atInstall.stdout}\n${atInstall.stderr}`).not.toBe("");

    const older = await olderAtRecorded(p);
    const off = await sessionStart(p, agent, older);
    expect(off.stdout).toBe("{}");
    expect(off.stderr).toContain(`rerail hook: Rerail needs Node 24 or newer and this is Node ${OLDER_NODE_VERSION}, so it is off in this session.`);

    // Node 24 or newer first on PATH, in a shell of the person's, then the step as the line wrote it.
    const node24First = { ...older, PATH: [dirname(process.execPath), older.PATH].join(delimiter) };
    const done = await inShell(shell, namedStep(off.stderr), p.root, node24First);
    expect(done.code, `${done.stdout}\n${done.stderr}`).toBe(0);
    const back = await sessionStart(p, agent, node24First);
    expect(back.note, back.stderr).not.toBe("");
    // A session the agent started before PATH changed gets it back too: the hook now runs the Node the step recorded.
    expect((await sessionStart(p, agent, older)).note).not.toBe("");
  }, 120_000);
});
