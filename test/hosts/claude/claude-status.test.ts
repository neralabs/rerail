import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { Readable, Writable } from "node:stream";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ClaudeStatusLine } from "../../../src/setup/index.js";

const mocks = vi.hoisted(() => ({ command: vi.fn(), presentation: vi.fn(), gitBash: vi.fn<() => string | undefined>() }));
vi.mock("../../../src/setup/claude-status-store.js", () => ({ readClaudeStatusCommand: mocks.command }));
vi.mock("../../../src/engine/git.js", async importOriginal => ({
  ...await importOriginal<Record<string, unknown>>(),
  gitBash: mocks.gitBash,
}));
vi.mock("../../../src/presentation/session-presentation.js", () => ({
  readPresentation: mocks.presentation,
  fallbackPresentation: (reason: string) => ({ text: reason }),
  formatTerminalStatus: (presentation: { text?: string }) => presentation.text ?? "Rerail | 2 rules | steered 1×",
}));
import { previousStatusSpawn, runClaudeStatus } from "../../../src/hosts/claude/claude-status.js";
import { removeFolder, runningIn } from "../../support/platform/remove-folder.js";

const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

/** Waits until `done` holds, asking every 100 ms, for at most `ms`. */
async function waitUntil(done: () => boolean, ms: number): Promise<void> {
  const deadline = Date.now() + ms;
  while (!done() && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 100));
}

function nodePredecessor(script: string): ClaudeStatusLine {
  return { type: "command", command: `${quote(process.execPath)} -e ${quote(script)}` };
}

async function render(input = Buffer.from('{"session_id":"claude-1","cwd":"/workspace"}'), settingsPath?: string): Promise<Buffer> {
  const chunks: Buffer[] = [];
  const output = new Writable({ write(chunk: Buffer, _encoding, done) { chunks.push(Buffer.from(chunk)); done(); } });
  await runClaudeStatus(Readable.from([input]), output, settingsPath);
  return Buffer.concat(chunks);
}

beforeEach(async () => {
  vi.clearAllMocks();
  mocks.command.mockReset().mockReturnValue(undefined);
  mocks.presentation.mockResolvedValue({ presence: "compact" });
  const git = await vi.importActual<typeof import("../../../src/engine/git.js")>("../../../src/engine/git.js");
  mocks.gitBash.mockReset().mockImplementation(() => git.gitBash());
});

const drives: string[] = [];
const roots: string[] = [];
afterEach(async () => {
  // A drive goes before its folder: Windows keeps a folder a drive maps.
  for (const drive of drives.splice(0)) execFileSync("subst", [drive, "/d"], { stdio: "ignore" });
  await Promise.all(roots.splice(0).map(async root => await removeFolder(root)));
});

/**
 * A folder of this test reached as the root of a free drive, holding the stand-ins `rerail-git\bin\bash.exe` and
 * `msys64\usr\bin\bash.exe` and an empty `session`, as a drive holding a person's bash is reached. Vitest runs test
 * files at once and every file shares the user's drive letters, so this file takes only X to Z (the PATH search's
 * tests take R to T, the start state's earlier-spelling tests U to W), and the next one where `subst` refuses a letter.
 */
async function driveWithBash(): Promise<{ root: string; drive: string }> {
  const root = await mkdtemp(join(tmpdir(), "rerail-claude-bash-"));
  roots.push(root);
  for (const folder of [join("rerail-git", "bin"), join("msys64", "usr", "bin")]) {
    await mkdir(join(root, folder), { recursive: true });
    await writeFile(join(root, folder, "bash.exe"), "");
  }
  await mkdir(join(root, "session"));
  for (const name of "XYZ") {
    const drive = `${name}:`;
    if (existsSync(`${drive}\\`)) continue;
    try {
      execFileSync("subst", [drive, root], { stdio: "ignore" });
      drives.push(drive);
      return { root, drive };
    } catch {
      // Taken since the check above: the next one.
    }
  }
  throw new Error("no free drive letter from X to Z");
}

/** How the previous status line is started on Windows, with `named` as CLAUDE_CODE_GIT_BASH_PATH, from `folder`. */
function spawnFrom(folder: string, command: string, named: string): ReturnType<typeof previousStatusSpawn> {
  const before = process.cwd();
  process.chdir(folder);
  try {
    return previousStatusSpawn(command, "win32", { CLAUDE_CODE_GIT_BASH_PATH: named });
  } finally {
    process.chdir(before);
  }
}

describe("composed Claude status", () => {
  it("selects the installed project predecessor independently of the global registration", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-claude-status-"));
    const store = await vi.importActual<typeof import("../../../src/setup/claude-status-store.js")>("../../../src/setup/claude-status-store.js");
    const globalSettingsPath = join(home, ".claude", "settings.json");
    const projectSettingsPath = join(home, "project", ".claude", "settings.local.json");
    try {
      for (const [settingsPath, label] of [[globalSettingsPath, "global"], [projectSettingsPath, "project"]] as const) {
        await store.writeClaudeStatusRegistration(home, {
          settingsPath,
          installedRoot: home,
          installedCommand: "rerail-status",
          previous: nodePredecessor(`process.stdout.write("${label} status")`),
        }, async () => {});
      }
      // Keep the real SQLite lookup and subprocess execution, isolating only the user's home.
      mocks.command.mockImplementation((_home: string | undefined, settingsPath?: string) =>
        store.readClaudeStatusCommand(home, settingsPath));
      expect((await render(undefined, projectSettingsPath)).toString()).toBe("project status\nRerail | 2 rules | steered 1×\n");
      expect((await render()).toString()).toBe("global status\nRerail | 2 rules | steered 1×\n");
      await store.writeClaudeStatusRegistration(home, {
        ...store.readClaudeStatusRegistration(home, projectSettingsPath)!, previousInherited: true,
      }, async () => {});
      for (const label of ["main", "worktree", "changed-worktree", "local-worktree"]) {
        const cwd = join(home, label === "main" ? "project" : "worktree");
        const path = join(cwd, ".claude", label.startsWith("local") ? "settings.local.json" : "settings.json");
        await mkdir(dirname(path), { recursive: true });
        await writeFile(path, JSON.stringify({ statusLine: { type: "command", command: `printf ${label}` } }));
        mocks.command.mockImplementation((_home: string | undefined, settingsPath?: string) =>
          store.readClaudeStatusCommand(home, settingsPath, cwd));
        expect((await render(undefined, projectSettingsPath)).toString()).toBe(`${label}\nRerail | 2 rules | steered 1×\n`);
      }
      await removeFolder(join(home, "worktree", ".claude"));
      await mkdir(dirname(globalSettingsPath), { recursive: true });
      await writeFile(globalSettingsPath, JSON.stringify({ statusLine: { type: "command", command: "rerail-status" } }));
      expect((await render(undefined, projectSettingsPath)).toString()).toBe("global status\nRerail | 2 rules | steered 1×\n");
    } finally {
      await removeFolder(home);
    }
  });

  it("forwards byte-exact stdin and preserves byte-exact prior status when Rerail is silent", async () => {
    mocks.command.mockReturnValue(nodePredecessor("process.stdin.pipe(process.stdout)"));
    mocks.presentation.mockResolvedValue({ presence: "silent" });
    const input = Buffer.from(' \n{"session_id":"claude-1","cwd":"/workspace","unrecognized":{"v":"é"}}  \n');
    expect(await render(input)).toEqual(input);
  });

  it("writes no Rerail line where the presentation has nothing to draw (Rerail is not on here, #298)", async () => {
    mocks.command.mockReturnValue(nodePredecessor("process.stdout.write('existing status')"));
    mocks.presentation.mockResolvedValue({ presence: "compact", status: "not-on-here", text: "" });
    expect((await render()).toString()).toBe("existing status");
  });

  it("identifies the session by the launch root when Claude has moved below it", async () => {
    await render(Buffer.from('{"session_id":"claude-1","cwd":"/workspace/src","workspace":{"project_dir":"/workspace"}}'));
    await render(Buffer.from('{"session_id":"claude-1","cwd":"/workspace/src","workspace":{"project_dir":"relative"}}'));
    expect(mocks.presentation.mock.calls.map(([request]) => request.cwd)).toEqual(["/workspace", "/workspace/src"]);
  });

  it("retains prior output on command failure and separates the Rerail line", async () => {
    mocks.command.mockReturnValue(nodePredecessor("process.stdout.write('existing\\u001b[32m status'); process.exitCode = 7"));
    const result = await render();
    expect(result.toString()).toBe("existing\u001b[32m status\nRerail | 2 rules | steered 1×\n");
  });

  it("retains emitted output when the prior command exceeds its execution budget", async () => {
    // Real subprocess integration: exercise the platform process-group timeout and retained stdout.
    mocks.command.mockReturnValue(nodePredecessor("process.stdout.write('slow status\\n'); setInterval(() => {}, 1000)"));
    const result = await render();
    expect(result.toString()).toBe("slow status\nRerail | 2 rules | steered 1×\n");
  });

  it("ends the prior command, not just its shell, once it exceeds its execution budget", async () => {
    const folder = await mkdtemp(join(tmpdir(), "rerail-claude-status-"));
    const pidFile = join(folder, "pid");
    try {
      // On Windows the command runs under Git Bash's launcher, which starts the real bash, which starts node. Git Bash
      // reads a backslash in its command line as an escape, so the path goes with forward slashes, which node takes.
      mocks.command.mockReturnValue(nodePredecessor(
        `require('node:fs').writeFileSync(${JSON.stringify(pidFile.replaceAll("\\", "/"))}, String(process.pid)); process.stdout.write('slow status\\n'); setInterval(() => {}, 1000)`,
      ));
      expect((await render()).toString()).toBe("slow status\nRerail | 2 rules | steered 1×\n");
      const pid = Number(await readFile(pidFile, "utf8"));
      const alive = (): boolean => {
        try {
          process.kill(pid, 0);
          return true;
        } catch {
          return false;
        }
      };
      // The status returns once it has asked for the tree's end, and the tree goes a moment later. On Windows taskkill
      // ends node, the real bash and Git Bash's launcher one after another, and this process hears of the launcher's
      // exit only after Windows has ended it, so the launcher, the shell this test started, can outlive node by tens of
      // milliseconds on a busy machine. Both are awaited, within one bound: the command, and every process this test
      // started that still runs in the folder.
      await waitUntil(() => !alive() && runningIn(folder).length === 0, 5_000);
      const survived = alive();
      if (survived) process.kill(pid);
      expect(survived).toBe(false);
      expect(runningIn(folder)).toEqual([]);
    } finally {
      // A command the test ended itself, above, takes its shell with it a moment later.
      await waitUntil(() => runningIn(folder).length === 0, 5_000);
      await removeFolder(folder);
    }
  }, 15_000);

  it("returns within its budget when the prior command leaves a process holding its output", async () => {
    // The grandchild leaves the command's process group and keeps the stdout pipe for 10 s.
    const script = "require('node:child_process').spawn(process.execPath, ['-e', 'setTimeout(() => {}, 10000)'], "
      + "{ detached: true, stdio: ['ignore', 'inherit', 'ignore'] }).unref(); process.stdout.write('held status')";
    mocks.command.mockReturnValue(nodePredecessor(script));
    const started = performance.now();
    const result = (await render()).toString();
    expect(performance.now() - started).toBeLessThan(5_000);
    expect(result).toBe("held status\nRerail | 2 rules | steered 1×\n");
  }, 15_000);

  it("runs the prior command on Windows through Git Bash, as Claude Code does, not cmd.exe", () => {
    const command = "bash ~/.claude/statusline.sh";
    const bash = String.raw`C:\Program Files\Git\bin\bash.exe`;
    expect(previousStatusSpawn(command, "win32", { CLAUDE_CODE_GIT_BASH_PATH: bash })).toEqual({ file: bash, args: ["-c", command], shell: false });
    expect(previousStatusSpawn(command, "linux", {})).toEqual({ file: command, args: [], shell: true });
  });

  it("finds Git Bash beside Git where CLAUDE_CODE_GIT_BASH_PATH is not an absolute path, which Rerail would not start", () => {
    const command = "bash ~/.claude/statusline.sh";
    expect(previousStatusSpawn(command, "win32", { CLAUDE_CODE_GIT_BASH_PATH: "bash.exe" })).toEqual(previousStatusSpawn(command, "win32", {}));
  });

  it("finds Git Bash beside Git where CLAUDE_CODE_GIT_BASH_PATH starts at the root of the current drive and names no file there", () => {
    const command = "bash ~/.claude/statusline.sh";
    for (const named of [String.raw`\rerail-no-such-folder\bin\bash.exe`, "/rerail-no-such-folder/bin/bash.exe"]) {
      expect(previousStatusSpawn(command, "win32", { CLAUDE_CODE_GIT_BASH_PATH: named }), named).toEqual(previousStatusSpawn(command, "win32", {}));
    }
  });

  // Windows reads a path without a drive on the drive of the current folder, as Claude Code does, and so did the start
  // before such a path was refused: the bash keeps its place there, named by its drive, Git Bash beside Git or not.
  it.runIf(process.platform === "win32")("on Windows starts the bash CLAUDE_CODE_GIT_BASH_PATH starts at the root of the current drive, by its path on that drive, where no Git Bash sits beside Git", async () => {
    const command = "bash ~/.claude/statusline.sh";
    const { drive } = await driveWithBash();
    mocks.gitBash.mockReturnValue(undefined);
    for (const named of [String.raw`\rerail-git\bin\bash.exe`, "/rerail-git/bin/bash.exe"]) {
      expect(spawnFrom(`${drive}\\session`, command, named), named).toEqual({ file: `${drive}\\rerail-git\\bin\\bash.exe`, args: ["-c", command], shell: false });
    }
  });

  it.runIf(process.platform === "win32")("on Windows keeps another bash CLAUDE_CODE_GIT_BASH_PATH starts at the root of the current drive over Git Bash beside Git", async () => {
    const command = "bash ~/.claude/statusline.sh";
    const { drive } = await driveWithBash();
    mocks.gitBash.mockReturnValue(String.raw`C:\Program Files\Git\bin\bash.exe`);
    expect(spawnFrom(`${drive}\\session`, command, String.raw`\msys64\usr\bin\bash.exe`)).toEqual({ file: `${drive}\\msys64\\usr\\bin\\bash.exe`, args: ["-c", command], shell: false });
  });

  it.runIf(process.platform === "win32")("on Windows finds Git Bash beside Git where CLAUDE_CODE_GIT_BASH_PATH starts at the root of the current drive and that drive holds no such file", async () => {
    const command = "bash ~/.claude/statusline.sh";
    const { root, drive } = await driveWithBash();
    const besideGit = String.raw`C:\Program Files\Git\bin\bash.exe`;
    mocks.gitBash.mockReturnValue(besideGit);
    // The same folder reached on the drive the temporary folder lies on, which holds no `\rerail-git`.
    expect(spawnFrom(join(root, "session"), command, String.raw`\rerail-git\bin\bash.exe`)).toEqual({ file: besideGit, args: ["-c", command], shell: false });
    // A bare name stays passed over beside a file it names in the current folder.
    expect(spawnFrom(`${drive}\\rerail-git\\bin`, command, "bash.exe")).toEqual({ file: besideGit, args: ["-c", command], shell: false });
  });

  // A file in the repository the session runs in is the repository's, not the person's bash: Git's own search skips such
  // a folder too, and so does the PATH search.
  it.runIf(process.platform === "win32")("on Windows never starts a bash CLAUDE_CODE_GIT_BASH_PATH names in a repository at the root of the current drive", async () => {
    const command = "bash ~/.claude/statusline.sh";
    const { root, drive } = await driveWithBash();
    await mkdir(join(root, ".git"));
    const besideGit = String.raw`C:\Program Files\Git\bin\bash.exe`;
    mocks.gitBash.mockReturnValue(besideGit);
    expect(spawnFrom(`${drive}\\session`, command, String.raw`\rerail-git\bin\bash.exe`)).toEqual({ file: besideGit, args: ["-c", command], shell: false });
    mocks.gitBash.mockReturnValue(undefined);
    expect(spawnFrom(`${drive}\\session`, command, String.raw`\rerail-git\bin\bash.exe`)).toEqual({ file: command, args: [], shell: true });
  });

  // On Linux and macOS a backslash is part of a file name, so there `\bash.exe` names a file in the current folder.
  it.skipIf(process.platform === "win32")("on Linux and macOS, playing Windows, passes over a CLAUDE_CODE_GIT_BASH_PATH such as \\bash.exe, which names a file in the current folder there", async () => {
    const command = "bash ~/.claude/statusline.sh";
    const here = await mkdtemp(join(tmpdir(), "rerail-claude-bash-"));
    roots.push(here);
    await writeFile(join(here, String.raw`\bash.exe`), "");
    const besideGit = String.raw`C:\Program Files\Git\bin\bash.exe`;
    mocks.gitBash.mockReturnValue(besideGit);
    expect(spawnFrom(here, command, String.raw`\bash.exe`)).toEqual({ file: besideGit, args: ["-c", command], shell: false });
  });

  it("draws Rerail's line when the prior command cannot be started at all", async () => {
    // Node refuses a command line holding a NUL byte before it starts anything, by throwing from spawn.
    mocks.command.mockReturnValue({ type: "command", command: "printf 'old\u0000status'" });
    expect((await render()).toString()).toBe("Rerail | 2 rules | steered 1×\n");
  });

  it.runIf(process.platform === "win32")("keeps a bash status line working on Windows once Rerail wraps it", async () => {
    mocks.command.mockReturnValue({ type: "command", command: "printf 'bash %s' \"$((1 + 1))\"" });
    expect((await render()).toString()).toBe("bash 2\nRerail | 2 rules | steered 1×\n");
  });

  it("keeps forwarding input beyond Rerail's JSON budget to the original command", async () => {
    const script = "let n=0; process.stdin.on('data', b => n+=b.length); process.stdin.on('end', () => process.stdout.write(String(n)))";
    mocks.command.mockReturnValue(nodePredecessor(script));
    const input = Buffer.alloc(1_048_577, 0x61);
    expect((await render(input)).toString()).toBe(`${input.length}\nsession unavailable\n`);
    expect(mocks.presentation).not.toHaveBeenCalled();
  });
});
