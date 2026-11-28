/**
 * One Claude Code session whose hooks name two model folders, against a daemon of this build and the hooks set-up
 * registers. First an update while one of the session's hooks waited for a busy daemon: the hook leaves its event
 * untaken with the model folder of the version it ran, a hook of the new version leaves its own behind it, the person
 * runs the stop and start the update asks for, and the new daemon checks the two together, then the hooks of the new
 * version, which name the new version's folder. Then
 * `RERAIL_MODEL_PATH` set as the session starts and unset when it is resumed. Every event of the session is recorded,
 * in the one runtime it opened in; before, each one after that runtime opened was refused (`Rerail session <id> is
 * already owned by a live runtime`), the hook answered `{}` with exit code 0, and nothing was recorded.
 *
 * Each case runs on a machine of its own, made inside the checkout's ignored `tmp/` folder: the two versions' packages
 * (hard links of the build's files under `<home>/versions/<version>/node_modules/@neralabs/rerail`, as the updater lays
 * them out) find the checkout's node_modules by Node's walk up the folders, on the volume their links need. The daemon
 * stops reading as one busy longer than a hook waits does: SIGSTOP, or NtSuspendProcess on Windows.
 */
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { chmodSync, existsSync, linkSync, mkdirSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { mkdir, mkdtemp } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import type { Writable } from "node:stream";
import { text } from "node:stream/consumers";
import Database from "better-sqlite3";
import { afterAll, describe, expect, it } from "vitest";
import { removeFolder } from "../../test/support/platform/remove-folder.js";
import { setUpTestRepository } from "../../test/support/repository-rules.js";

const WINDOWS = process.platform === "win32";
const PACKAGE_ROOT = resolve(".");
const SYSTEM_ROOT = process.env.SystemRoot ?? "C:\\Windows";
const CMD = join(SYSTEM_ROOT, "System32", "cmd.exe");
const POWERSHELL = join(SYSTEM_ROOT, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
const MANIFEST = JSON.parse(readFileSync(join(PACKAGE_ROOT, "package.json"), "utf8")) as Record<string, unknown> & { version: string; files: string[] };
/** The version built, which set-up installs, and the next patch, which npm offers the update. */
const FIRST = MANIFEST.version;
const NEXT = MANIFEST.version.replace(/^(\d+\.\d+\.)(\d+).*$/u, (_whole, head: string, patch: string) => `${head}${Number(patch) + 1}`);
const folders: string[] = [];

afterAll(async () => {
  await Promise.all(folders.splice(0).map(folder => removeFolder(folder)));
});

interface Ran {
  code: number | null;
  stdout: string;
  stderr: string;
}

/** `command` run to its end, with `input` on its stdin; with `verbatim`, Windows gets `args` joined as written. */
async function run(command: string, args: readonly string[], cwd: string, env: NodeJS.ProcessEnv, input = "", verbatim = false): Promise<Ran> {
  const child = spawn(command, [...args], { cwd, env, stdio: ["pipe", "pipe", "pipe"], windowsHide: true, windowsVerbatimArguments: verbatim });
  const [stdout, stderr, [code]] = await Promise.all([text(child.stdout), text(child.stderr), once(child, "close") as Promise<[number | null]>, fed(child.stdin, input)]);
  return { code, stdout, stderr };
}

/** Writes `input` to a child's stdin and closes it; a child that ends without reading it (EPIPE) is no failure. */
async function fed(stdin: Writable, input: string): Promise<void> {
  const closed = Promise.withResolvers<void>();
  stdin.on("error", (error: NodeJS.ErrnoException) => (error.code === "EPIPE" ? closed.resolve() : closed.reject(error)));
  stdin.on("close", () => closed.resolve());
  stdin.end(input);
  await closed.promise;
}

/** Hard links of `from` at `to`, folder by folder: each file has a path of its own, as a copy's has. */
function linkTree(from: string, to: string): void {
  if (statSync(from).isDirectory()) {
    mkdirSync(to, { recursive: true });
    for (const name of readdirSync(from)) linkTree(join(from, name), join(to, name));
    return;
  }
  mkdirSync(dirname(to), { recursive: true });
  linkSync(from, to);
}

/** The build at `root` as npm packs it (its `files`), with this build's engine in `native/` and `version` as its own. */
function placePackage(root: string, version: string): void {
  for (const entry of MANIFEST.files.filter(file => !file.startsWith("!"))) {
    const [folder, suffix] = entry.includes("*") ? [dirname(entry), entry.slice(entry.indexOf("*") + 1)] : [undefined, undefined];
    if (folder === undefined) {
      if (existsSync(join(PACKAGE_ROOT, entry))) linkTree(join(PACKAGE_ROOT, entry), join(root, entry));
      continue;
    }
    const names = existsSync(join(PACKAGE_ROOT, folder)) ? readdirSync(join(PACKAGE_ROOT, folder)).filter(name => name.endsWith(suffix!)) : [];
    for (const name of names) linkTree(join(PACKAGE_ROOT, folder, name), join(root, folder, name));
  }
  for (const name of readdirSync(join(PACKAGE_ROOT, "native")).filter(file => /^rerail_engine.*\.node$/u.test(file))) {
    linkTree(join(PACKAGE_ROOT, "native", name), join(root, "native", name));
  }
  writeFileSync(join(root, "package.json"), `${JSON.stringify({ ...MANIFEST, version }, null, 2)}\n`);
}

/** Stops process `pid` reading, as a daemon busy longer than a hook waits, or lets it go on; it lives, and its socket takes requests unanswered. */
function hold(pid: number, on: boolean): void {
  if (!WINDOWS) {
    process.kill(pid, on ? "SIGSTOP" : "SIGCONT");
    return;
  }
  const call = on ? "NtSuspendProcess" : "NtResumeProcess";
  const script = `$t = Add-Type -PassThru -Name N${call} -Namespace P -MemberDefinition '[DllImport("ntdll.dll")] public static extern int ${call}(IntPtr h);'; `
    + `$p = [System.Diagnostics.Process]::GetProcessById(${pid}); exit $t::${call}($p.Handle)`;
  const ran = spawnSync(POWERSHELL, ["-NoProfile", "-NonInteractive", "-Command", script], { encoding: "utf8", windowsHide: true });
  if (ran.status !== 0) throw new Error(`${call} of ${pid}: ${String(ran.status)} ${ran.stderr}`);
}

/** Whether process `pid` still runs. */
function running(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/**
 * A person's machine: a Rerail home `h`, a user home `u` that Claude Code's settings go in, a Git repository `app` with
 * one commit, and `bin`, where npm's stand-in goes. Its shell has none of the test run's Rerail variables: autostart
 * is on, and the home's own content store is used.
 */
class Machine {
  readonly home: string;
  readonly user: string;
  readonly app: string;
  readonly bin: string;
  readonly env: NodeJS.ProcessEnv;

  constructor(readonly root: string) {
    this.home = join(root, "h");
    this.user = join(root, "u");
    this.app = join(root, "app");
    this.bin = join(root, "bin");
    for (const folder of [join(this.user, ".claude"), this.app, this.bin]) mkdirSync(folder, { recursive: true });
    const kept = Object.entries(process.env).filter(([name]) => !/^(?:RERAIL_|NPM_|VITEST|NODE_OPTIONS$|NODE_ENV$|CLAUDE_)/iu.test(name));
    this.env = {
      ...Object.fromEntries(kept), RERAIL_HOME: this.home, HOME: this.user, USERPROFILE: this.user, BROWSER: "none",
      RERAIL_DEVICE_SECRET_STORE: "file", RERAIL_MODIFY_PATH: "0", RERAIL_EXPERIMENTAL_TRAY_ICON: "0",
      // npm as the updater runs it: on Windows the script `npm_execpath` names, elsewhere the `npm` first on PATH.
      ...(WINDOWS ? { npm_execpath: join(this.bin, "npm-cli.js") } : { PATH: `${this.bin}:${process.env.PATH ?? ""}` }),
    };
  }

  static async made(): Promise<Machine> {
    await mkdir(join(PACKAGE_ROOT, "tmp"), { recursive: true });
    const root = realpathSync.native(await mkdtemp(join(PACKAGE_ROOT, "tmp", "one-runtime-")));
    folders.push(root);
    const machine = new Machine(root);
    writeFileSync(join(machine.app, "README.md"), "app\n");
    for (const args of [["init", "-q"], ["add", "."], ["-c", "user.name=R", "-c", "user.email=r@example.invalid", "commit", "-q", "-m", "seed"]]) {
      const git = await run("git", args, machine.app, machine.env);
      expect(git.code, git.stderr).toBe(0);
    }
    return machine;
  }

  /** `<home>/versions/<version>/node_modules/@neralabs/rerail`, placed. */
  placeVersion(version: string): string {
    const root = this.packageOf(version);
    placePackage(root, version);
    return root;
  }

  packageOf(version: string): string {
    return join(this.home, "versions", version, "node_modules", "@neralabs", "rerail");
  }

  /** Turns Rerail on in the repository, in the home's database, as set-up leaves it once a person says yes. */
  startRepository(): void {
    const workerHome = process.env.RERAIL_HOME;
    process.env.RERAIL_HOME = this.home;
    try {
      setUpTestRepository(this.app, {}, join(this.home, "rerail.sqlite"));
    } finally {
      if (workerHome === undefined) delete process.env.RERAIL_HOME;
      else process.env.RERAIL_HOME = workerHome;
    }
  }

  /** A registry whose newest Rerail is `version`, answering `npm view` alone. */
  npmOffers(version: string): void {
    const answer = `process.stdout.write(${JSON.stringify(`${JSON.stringify(version)}\n`)});`;
    writeFileSync(join(this.bin, "npm-cli.js"), `if (process.argv.includes("view")) ${answer} else process.exit(1);\n`);
    writeFileSync(join(this.bin, "package.json"), `{"type":"commonjs"}\n`);
    if (WINDOWS) return;
    writeFileSync(join(this.bin, "npm"), `#!/bin/sh\nexec "${process.execPath}" "${join(this.bin, "npm-cli.js")}" "$@"\n`);
    chmodSync(join(this.bin, "npm"), 0o755);
  }

  /** `rerail …` as a person's shell runs it: the launcher set-up wrote in `<home>/bin`. */
  async rerail(args: readonly string[]): Promise<Ran> {
    if (!WINDOWS) return await run(join(this.home, "bin", "rerail"), args, this.app, this.env);
    return await run(CMD, ["/d", "/s", "/c", `""${join(this.home, "bin", "rerail.cmd")}" ${args.join(" ")}"`], this.app, this.env, "", true);
  }

  /** The CLI of the package at `packageRoot`, as `npx @neralabs/rerail` runs it. */
  async cliOf(packageRoot: string, args: readonly string[]): Promise<Ran> {
    return await run(process.execPath, [join(packageRoot, "dist", "cli", "cli.js"), ...args], this.app, this.env);
  }

  /** One hook event as Claude Code sends it: the command `~/.claude/settings.json` holds for `event`, the event on stdin. */
  async hook(event: string, payload: Record<string, unknown>, extra: NodeJS.ProcessEnv = {}): Promise<Ran> {
    const settings = JSON.parse(readFileSync(join(this.user, ".claude", "settings.json"), "utf8")) as { hooks: Record<string, Array<{ hooks: Array<{ command: string; args?: string[] }> }>> };
    const entry = settings.hooks[event]?.flatMap(group => group.hooks).find(each => [each.command, ...(each.args ?? [])].join(" ").includes("claude-hook"));
    if (entry === undefined) throw new Error(`no Rerail hook for ${event}`);
    const env = { ...this.env, CLAUDE_PROJECT_DIR: this.app, ...extra };
    const input = JSON.stringify({ cwd: this.app, hook_event_name: event, ...payload });
    if (entry.args !== undefined) return await run(entry.command, entry.args, this.app, env, input);
    return WINDOWS ? await run(CMD, ["/d", "/s", "/c", `"${entry.command}"`], this.app, env, input, true) : await run("/bin/sh", ["-c", entry.command], this.app, env, input);
  }

  daemonPid(): number {
    return (JSON.parse(readFileSync(join(this.home, "daemon.json"), "utf8")) as { pid: number }).pid;
  }

  untaken(): string[] {
    const folder = join(this.home, "untaken-events");
    return existsSync(folder) ? readdirSync(folder).filter(name => name.endsWith(".json")) : [];
  }

  /** The ids of the tool calls the home's database records for `sessionId`, in the order they were recorded. */
  recordedCalls(sessionId: string): string[] {
    const database = new Database(join(this.home, "rerail.sqlite"), { readonly: true });
    try {
      const rows = database.prepare("SELECT event_id AS eventId FROM events WHERE session_id = ? AND kind = 'call' ORDER BY sequence").all(sessionId) as Array<{ eventId: string }>;
      return rows.map(row => row.eventId.slice(`${sessionId}:tool-call:`.length));
    } finally {
      database.close();
    }
  }

  /** Stops the daemon `rerail daemon stop` finds and ends `held`, a daemon a failed case left stopped, by its own pid. */
  async stopAll(held: number | undefined): Promise<void> {
    if (held !== undefined && running(held)) hold(held, false);
    await this.rerail(["daemon", "stop"]);
    if (held !== undefined && running(held)) process.kill(held, "SIGKILL");
  }
}

/** A Claude Code session's hook events, by id: its start, its prompts and its Reads, each Read's tool call id kept. */
class Session {
  readonly calls: string[] = [];
  #prompts = 0;

  constructor(readonly machine: Machine, readonly id: string) {}

  async start(source: "startup" | "resume", extra: NodeJS.ProcessEnv = {}): Promise<Ran> {
    return await this.machine.hook("SessionStart", { session_id: this.id, source }, extra);
  }

  async prompt(extra: NodeJS.ProcessEnv = {}): Promise<Ran> {
    this.#prompts += 1;
    return await this.machine.hook("UserPromptSubmit", { session_id: this.id, prompt_id: `p-${this.#prompts}`, prompt: "Go on." }, extra);
  }

  async read(extra: NodeJS.ProcessEnv = {}): Promise<Ran> {
    const call = `toolu_${this.calls.length + 1}`;
    this.calls.push(call);
    const input = { file_path: join(this.machine.app, "README.md") };
    return await this.machine.hook("PreToolUse", { session_id: this.id, tool_name: "Read", tool_use_id: call, tool_input: input }, extra);
  }
}

/** What a person sees of a hook: its exit code and its stderr. */
const seen = (ran: Ran) => ({ code: ran.code, stderr: ran.stderr });
const ANSWERED = { code: 0, stderr: "" };

describe("one session whose hooks name two model folders, against a daemon", () => {
  it("records every hook of a session after an update while one of its events waited for a busy daemon, and the events that waited", async () => {
    const m = await Machine.made();
    const first = m.placeVersion(FIRST);
    m.placeVersion(NEXT);
    const setup = await m.cliOf(first, ["setup", "--json"]);
    expect(setup.code, setup.stderr).toBe(0);
    const install = await m.rerail(["install", "--agents", "claude-code"]);
    expect(install.code, install.stderr).toBe(0);
    m.startRepository();
    let held: number | undefined;
    try {
      expect((await m.rerail(["daemon", "start"])).code).toBe(0);
      const s = new Session(m, "held-across-update");
      expect([seen(await s.start("startup")), seen(await s.prompt()), seen(await s.read())]).toEqual([ANSWERED, ANSWERED, ANSWERED]);
      held = m.daemonPid();
      hold(held, true);
      // The hook waits its whole wait for the daemon, then leaves its event with the first version's model folder.
      expect(seen(await s.read()).code).toBe(0);
      expect(m.untaken()).toHaveLength(1);
      m.npmOffers(NEXT);
      const update = await m.rerail(["update"]);
      expect(update.code, `${update.stdout}\n${update.stderr}`).toBe(0);
      expect(realpathSync.native(join(m.home, "current"))).toBe(realpathSync.native(m.packageOf(NEXT)));
      // A hook of the next version before the stop: the event that waited is still the stopped daemon's, so this hook
      // leaves its event behind it at once, with the next version's model folder. The new daemon's first pass hands on
      // both together.
      expect(seen(await s.read()).code).toBe(0);
      expect(m.untaken()).toHaveLength(2);
      expect((await m.rerail(["daemon", "stop"])).code).toBe(0);
      const restarted = await m.rerail(["daemon", "start"]);
      expect(restarted.stdout, restarted.stderr).toContain(`Rerail daemon ${NEXT} running`);
      // The hooks now run the next version, whose model folder is its own package's.
      expect([seen(await s.read()), seen(await s.read()), seen(await s.read())]).toEqual([ANSWERED, ANSWERED, ANSWERED]);
      expect(m.recordedCalls(s.id)).toEqual(s.calls);
    } finally {
      await m.stopAll(held);
    }
  }, 300_000);

  it("records every hook of a session started with RERAIL_MODEL_PATH and resumed without it", async () => {
    const m = await Machine.made();
    const model = join(m.root, "model");
    linkTree(join(PACKAGE_ROOT, "model"), model);
    const install = await m.cliOf(PACKAGE_ROOT, ["install", "--agents", "claude-code", "--root", PACKAGE_ROOT]);
    expect(install.code, install.stderr).toBe(0);
    m.startRepository();
    try {
      expect((await m.rerail(["daemon", "start"])).code).toBe(0);
      const s = new Session(m, "model-path-then-none");
      const withIt = { RERAIL_MODEL_PATH: model };
      expect([seen(await s.start("startup", withIt)), seen(await s.prompt(withIt)), seen(await s.read(withIt))]).toEqual([ANSWERED, ANSWERED, ANSWERED]);
      // `claude --resume` in a shell that no longer sets it.
      expect([seen(await s.start("resume")), seen(await s.read()), seen(await s.read())]).toEqual([ANSWERED, ANSWERED, ANSWERED]);
      expect(m.recordedCalls(s.id)).toEqual(s.calls);
    } finally {
      await m.stopAll(undefined);
    }
  }, 120_000);
});
