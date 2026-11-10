import { mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { Readable, Writable } from "node:stream";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ClaudeStatusLine } from "../../../src/setup/index.js";

const mocks = vi.hoisted(() => ({ command: vi.fn(), presentation: vi.fn() }));
vi.mock("../../../src/setup/claude-status-store.js", () => ({ readClaudeStatusCommand: mocks.command }));
vi.mock("../../../src/presentation/session-presentation.js", () => ({
  readPresentation: mocks.presentation,
  fallbackPresentation: (reason: string) => ({ text: reason }),
  formatTerminalStatus: (presentation: { text?: string }) => presentation.text ?? "Rerail | 2 rules | steered 1×",
}));
import { previousStatusSpawn, runClaudeStatus } from "../../../src/hosts/claude/claude-status.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

function nodePredecessor(script: string): ClaudeStatusLine {
  return { type: "command", command: `${quote(process.execPath)} -e ${quote(script)}` };
}

async function render(input = Buffer.from('{"session_id":"claude-1","cwd":"/workspace"}'), settingsPath?: string): Promise<Buffer> {
  const chunks: Buffer[] = [];
  const output = new Writable({ write(chunk: Buffer, _encoding, done) { chunks.push(Buffer.from(chunk)); done(); } });
  await runClaudeStatus(Readable.from([input]), output, settingsPath);
  return Buffer.concat(chunks);
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.command.mockReset().mockReturnValue(undefined);
  mocks.presentation.mockResolvedValue({ presence: "compact" });
});

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
      const deadline = Date.now() + 5_000;
      while (alive() && Date.now() < deadline) await new Promise(resolve => setTimeout(resolve, 100));
      const survived = alive();
      if (survived) process.kill(pid);
      expect(survived).toBe(false);
    } finally {
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
