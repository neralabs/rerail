import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { Readable, Writable } from "node:stream";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type * as ClaudeStatusStore from "../src/claude-status-store.js";

const mocks = vi.hoisted(() => ({ command: vi.fn(), presentation: vi.fn() }));
vi.mock("../src/claude-status-store.js", () => ({ readClaudeStatusCommand: mocks.command }));
vi.mock("../src/session-presentation.js", () => ({
  readPresentation: mocks.presentation,
  formatTerminalStatus: () => "Rerail | 2 rules | steered 1×",
}));
import { runClaudeStatus } from "../src/claude-status.js";

const quote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

function nodePredecessor(script: string): ClaudeStatusStore.ClaudeStatusLine {
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
    const store = await vi.importActual<typeof ClaudeStatusStore>("../src/claude-status-store.js");
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
      await rm(join(home, "worktree", ".claude"), { recursive: true });
      await mkdir(dirname(globalSettingsPath), { recursive: true });
      await writeFile(globalSettingsPath, JSON.stringify({ statusLine: { type: "command", command: "rerail-status" } }));
      expect((await render(undefined, projectSettingsPath)).toString()).toBe("global status\nRerail | 2 rules | steered 1×\n");
    } finally {
      await rm(home, { recursive: true, force: true });
    }
  });

  it("forwards byte-exact stdin and preserves byte-exact prior status when Rerail is silent", async () => {
    mocks.command.mockReturnValue(nodePredecessor("process.stdin.pipe(process.stdout)"));
    mocks.presentation.mockResolvedValue({ presence: "silent" });
    const input = Buffer.from(' \n{"session_id":"claude-1","cwd":"/workspace","unrecognized":{"v":"é"}}  \n');
    expect(await render(input)).toEqual(input);
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

  it("keeps forwarding input beyond Rerail's JSON budget to the original command", async () => {
    const script = "let n=0; process.stdin.on('data', b => n+=b.length); process.stdin.on('end', () => process.stdout.write(String(n)))";
    mocks.command.mockReturnValue(nodePredecessor(script));
    const input = Buffer.alloc(1_048_577, 0x61);
    expect((await render(input)).toString()).toMatch(new RegExp(`^${input.length}\\n\\w+ \\| session unavailable\\n$`, "u"));
    expect(mocks.presentation).not.toHaveBeenCalled();
  });
});
