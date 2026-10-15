import { execFile } from "node:child_process";
import { afterEach, describe, expect, it, vi } from "vitest";
import { lstat, mkdir, mkdtemp, readdir, readFile, readlink, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";
import { sql } from "drizzle-orm";
import { openSqliteDatabase } from "../../src/engine/index.js";
import {
  CURSOR_HOOK_EVENTS,
  HARNESSES,
  detectHarnesses,
  registerHarnesses,
  unregisterHarnesses,
  type Harness,
  type RegistrationChange,
} from "../../src/setup/harness-registration.js";
import {
  claudeStatusDatabasePath,
  readClaudeStatusCommand,
  readClaudeStatusRegistration,
  writeClaudeStatusRegistration,
} from "../../src/setup/claude-status-store.js";
import { MANAGED_SKILL_MARKER, RERAIL_COMMANDS, readRepoQuestion, renderQuestion, shellQuote } from "../../src/harness/index.js";
import { runInstallCommand, setupAnswerCommand } from "../../src/cli/index.js";
import { ManagedUpdater } from "../../src/setup/updater.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

interface Fixture {
  home: string;
  root: string;
  nodePath: string;
}

async function fixture(): Promise<Fixture> {
  const base = await mkdtemp(join(tmpdir(), "rerail-registration-"));
  temporaryDirectories.push(base);
  const root = join(base, "node_modules", "@neralabs", "rerail");
  await mkdir(join(root, "dist"), { recursive: true });
  await writeFile(join(root, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "1.0.0" }));
  const home = join(base, "home");
  await mkdir(home);
  return { home, root, nodePath: "/opt/node/bin/node" };
}

/** Another install's package root beside `home`: `dist/` and a package.json named `name`, this package's own by default. */
async function packageInstall(options: Fixture, directory: string, name?: string): Promise<string> {
  const root = join(dirname(options.home), directory, "current");
  await mkdir(join(root, "dist"), { recursive: true });
  const own = JSON.parse(await readFile(join(options.root, "package.json"), "utf8")) as { name: string };
  await writeFile(join(root, "package.json"), JSON.stringify({ name: name ?? own.name, version: "0.1.0" }));
  return root;
}

async function seed(path: string, value: unknown): Promise<string> {
  const text = `${JSON.stringify(value, null, 2)}\n`;
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, text);
  return text;
}

async function readJson(path: string): Promise<Record<string, unknown>> {
  return JSON.parse(await readFile(path, "utf8")) as Record<string, unknown>;
}

/** The registered hook line: the recorded node, with node from PATH as the fallback once it is gone. */
function hookLine(nodePath: string, root: string, executable: string): string {
  return `sh -c 'if [ -x "$0" ]; then exec "$0" "$1"; fi; exec node "$1"' ${nodePath} ${join(root, "dist", executable)}`;
}

async function backups(path: string): Promise<string[]> {
  const name = path.slice(dirname(path).length + 1);
  return (await readdir(dirname(path))).filter((entry) => entry.startsWith(`${name}.`) && entry.endsWith(".bak"));
}

async function claudeFixture(settings?: unknown) {
  const options = { ...await fixture(), harnesses: ["claude-code"] as const };
  const path = join(options.home, ".claude", "settings.json");
  const original = settings === undefined ? undefined : await seed(path, settings);
  return { options, path, original };
}

const git = promisify(execFile);

async function worktreeFixture() {
  const { options, path: globalPath } = await claudeFixture();
  const base = await realpath(dirname(options.home));
  const main = join(base, "main");
  const workspaceDir = join(base, "worktree");
  await mkdir(main);
  await git("git", ["init", "-q", main]);
  await git("git", ["-c", "user.name=Rerail Test", "-c", "user.email=rerail@example.invalid", "commit", "--allow-empty", "-qm", "initial"], { cwd: main });
  await git("git", ["worktree", "add", "--detach", workspaceDir], { cwd: main });
  return {
    options: { ...options, main, workspaceDir },
    globalPath,
    rootPath: join(main, ".claude", "settings.local.json"),
    cwdPath: join(workspaceDir, ".claude", "settings.local.json"),
    sharedPath: join(workspaceDir, ".claude", "settings.json"),
    mainSharedPath: join(main, ".claude", "settings.json"),
  };
}

function change(changes: readonly RegistrationChange[], harness: Harness, suffix: string): RegistrationChange {
  const found = changes.find((entry) => entry.harness === harness && entry.path.endsWith(suffix));
  if (found === undefined) throw new Error(`no change for ${harness} ${suffix}`);
  return found;
}

describe("detectHarnesses", () => {
  it("reports harnesses whose config directory exists, in canonical order", async () => {
    const { home } = await fixture();
    expect(detectHarnesses(home)).toEqual([]);
    await mkdir(join(home, ".cursor"));
    await mkdir(join(home, ".claude"));
    await mkdir(join(home, ".pi", "agent"), { recursive: true });
    await mkdir(join(home, ".omp"));
    expect(detectHarnesses(home)).toEqual(["pi", "claude-code", "cursor"]);
  });

  it("registers only detected harnesses and reports the rest as skipped", async () => {
    const { home, root, nodePath } = await fixture();
    await mkdir(join(home, ".codex"));
    const report = await registerHarnesses({ root, home, nodePath });
    expect(report.changes.map((entry) => [entry.harness, entry.action])).toEqual([
      ["omp", "skipped"],
      ["pi", "skipped"],
      ["claude-code", "skipped"],
      ["codex", "created"],
      ...RERAIL_COMMANDS.map(() => ["codex", "created"]),
      ["cursor", "skipped"],
    ]);
    expect(report.notes.some((note) => note.includes("/hooks"))).toBe(true);
    const hooks = (await readJson(join(home, ".codex", "hooks.json"))).hooks as Record<string, unknown[]>;
    expect(Object.keys(hooks)).toEqual(["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"]);
    expect(hooks.PreToolUse).toEqual([
      { hooks: [{ type: "command", command: hookLine(nodePath, root, "hosts/codex/codex-hook.js") }] },
    ]);
    expect(hooks.PostToolUse).toEqual([
      { matcher: "", hooks: [{ type: "command", command: hookLine(nodePath, root, "hosts/codex/codex-hook.js") }] },
    ]);
  });

  it("runs the recorded node first and falls back to node on PATH when it is gone", async () => {
    const { home, root, nodePath } = await fixture();
    await mkdir(join(home, ".codex"));
    await registerHarnesses({ root, home, nodePath, harnesses: ["codex"] });
    const hooks = (await readJson(join(home, ".codex", "hooks.json"))).hooks as Record<
      string,
      Array<{ hooks: Array<{ command: string }> }>
    >;
    const command = hooks.SessionStart?.[0]?.hooks[0]?.command;
    const hookPath = join(root, "dist", "hosts", "codex", "codex-hook.js");
    // The test guards the first exec, so a hook that exits non-zero is never run twice.
    expect(command).toBe(`sh -c 'if [ -x "$0" ]; then exec "$0" "$1"; fi; exec node "$1"' ${nodePath} ${hookPath}`);
    // Unregister still finds the line: it carries the <root>/dist/ marker.
    expect(command).toContain(`${join(root, "dist")}/`);
  });

  it("quotes Windows hook commands without POSIX backslash escaping", async () => {
    const { home, root } = await fixture();
    const nodePath = String.raw`C:\Program Files\nodejs\node.exe`;
    await registerHarnesses({ root, home, nodePath, platform: "win32", harnesses: ["codex"] });
    const hooks = (await readJson(join(home, ".codex", "hooks.json"))).hooks as Record<
      string,
      Array<{ hooks: Array<{ command: string }> }>
    >;
    const command = hooks.SessionStart?.[0]?.hooks[0]?.command;
    const hookPath = join(root, "dist", "hosts", "codex", "codex-hook.js");
    // cmd runs the recorded node when it is still installed, and node from PATH otherwise.
    expect(command).toBe(`cmd /d /c "if exist "${nodePath}" ("${nodePath}" "${hookPath}") else (node "${hookPath}")"`);
  });
});

describe("Claude Code settings.json", () => {
  const foreign = {
    permissions: { allow: ["Bash(git:*)"] },
    theme: "dark",
    hooks: {
      PreToolUse: [
        { matcher: "", hooks: [{ type: "command", command: "/usr/bin/node /opt/nerling/dist/hooks/main.js" }] },
        { matcher: "*", hooks: [{ type: "command", command: "/opt/orca/hook.sh", timeout: 10 }] },
      ],
      Stop: [{ hooks: [{ type: "command", command: "/opt/orca/hook.sh", timeout: 10 }] }],
    },
  };

  it("merges rerail groups, is idempotent, and unregister restores the original bytes", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".claude", "settings.json");
    const original = await seed(path, foreign);
    const command = hookLine(nodePath, root, "hosts/claude/claude-hook.js");

    const first = await registerHarnesses({ root, home, nodePath, harnesses: ["claude-code"] });
    expect(change(first.changes, "claude-code", "settings.json").action).toBe("updated");
    expect(await backups(path)).toHaveLength(1);

    const settings = await readJson(path);
    expect(settings.permissions).toEqual(foreign.permissions);
    expect(settings.theme).toBe("dark");
    const hooks = settings.hooks as Record<string, unknown[]>;
    expect(Object.keys(hooks).sort()).toEqual(
      [
        "MessageDisplay",
        "PostToolUse",
        "PostToolUseFailure",
        "PreToolUse",
        "SessionEnd",
        "SessionStart",
        "Stop",
        "StopFailure",
        "UserPromptSubmit",
      ].sort(),
    );
    expect(hooks.PreToolUse).toEqual([
      ...foreign.hooks.PreToolUse,
      { matcher: "", hooks: [{ type: "command", command }] },
    ]);
    expect(hooks.Stop).toEqual([...foreign.hooks.Stop, { hooks: [{ type: "command", command }] }]);
    expect(hooks.SessionStart).toEqual([{ hooks: [{ type: "command", command }] }]);
    expect(hooks.PostToolUseFailure).toEqual([{ matcher: "", hooks: [{ type: "command", command }] }]);

    const second = await registerHarnesses({ root, home, nodePath, harnesses: ["claude-code"] });
    expect(change(second.changes, "claude-code", "settings.json").action).toBe("unchanged");
    expect(await backups(path)).toHaveLength(1);

    const removed = await unregisterHarnesses({ root, home, nodePath, harnesses: ["claude-code"] });
    expect(change(removed.changes, "claude-code", "settings.json").action).toBe("updated");
    expect(await readFile(path, "utf8")).toBe(original);

    const again = await unregisterHarnesses({ root, home, nodePath, harnesses: ["claude-code"] });
    expect(change(again.changes, "claude-code", "settings.json").action).toBe("unchanged");
  });

  it("creates settings.json when absent and unregister leaves an empty object", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".claude", "settings.json");
    const report = await registerHarnesses({ root, home, nodePath, harnesses: ["claude-code"] });
    expect(change(report.changes, "claude-code", "settings.json").action).toBe("created");
    expect(await backups(path)).toHaveLength(0);
    await unregisterHarnesses({ root, home, nodePath, harnesses: ["claude-code"] });
    expect(await readJson(path)).toEqual({});
  });

  it("replaces a stale registration that points at an older root", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".claude", "settings.json");
    // The pre-fallback command shape an older rerail wrote; the marker still identifies it as ours.
    const stale = `${nodePath} ${join(root, "dist", "hosts", "claude", "claude-hook.js")}`;
    await seed(path, {
      hooks: {
        SessionEnd: [{ hooks: [{ type: "command", command: stale }] }],
        Interrupt: [{ hooks: [{ type: "command", command: stale }] }],
      },
    });
    const newNode = "/opt/node-22/bin/node";
    await registerHarnesses({ root, home, nodePath: newNode, harnesses: ["claude-code"] });
    const hooks = (await readJson(path)).hooks as Record<string, unknown>;
    expect(hooks.Interrupt).toBeUndefined();
    expect(hooks.SessionEnd).toEqual([
      { hooks: [{ type: "command", command: hookLine(newNode, root, "hosts/claude/claude-hook.js") }] },
    ]);
  });

  it("registers Windows hooks through the exec-form launcher and replaces the cmd wrapper Git Bash mangles", async () => {
    const { home, root } = await fixture();
    const nodePath = String.raw`C:\Program Files\nodejs\node.exe`;
    const hookPath = join(root, "dist", "hosts", "claude", "claude-hook.js");
    const launcherPath = join(root, "dist", "harness", "hook-launcher.js");
    const path = join(home, ".claude", "settings.json");
    // Claude Code passes shell-form hooks to Git Bash, which turns `/d /c` into drive paths so cmd reads the event JSON as commands.
    const cmdWrapper = `cmd /d /c "if exist "${nodePath}" ("${nodePath}" "${hookPath}") else (node "${hookPath}")"`;
    await seed(path, { hooks: { ...foreign.hooks, SessionStart: [{ hooks: [{ type: "command", command: cmdWrapper }] }] } });
    const windows = { root, home, nodePath, platform: "win32" as const, harnesses: ["claude-code"] as const };

    await registerHarnesses(windows);
    // `node` from PATH keeps the hook alive after a version manager removes the recorded node.
    const execEntry = { type: "command", command: "node", args: [launcherPath, nodePath, hookPath] };
    const hooks = (await readJson(path)).hooks as Record<string, unknown[]>;
    expect(hooks.SessionStart).toEqual([{ hooks: [execEntry] }]);
    expect(hooks.PreToolUse).toEqual([...foreign.hooks.PreToolUse, { matcher: "", hooks: [execEntry] }]);
    expect(JSON.stringify(hooks)).not.toContain("cmd /d /c");
    expect(change((await registerHarnesses(windows)).changes, "claude-code", "settings.json").action).toBe("unchanged");

    const upgradedNode = String.raw`C:\Users\dev\AppData\Local\nvm\v24.9.0\node.exe`;
    await registerHarnesses({ ...windows, nodePath: upgradedNode });
    const upgraded = (await readJson(path)).hooks as Record<string, unknown[]>;
    expect(upgraded.SessionStart).toEqual([{ hooks: [{ ...execEntry, args: [launcherPath, upgradedNode, hookPath] }] }]);

    await unregisterHarnesses(windows);
    expect((await readJson(path)).hooks).toEqual(foreign.hooks);
  });

  it("aborts on invalid JSON without writing or backing up", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".claude", "settings.json");
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, "{ not json");
    await expect(registerHarnesses({ root, home, nodePath, harnesses: ["claude-code"] })).rejects.toThrow(path);
    expect(await readFile(path, "utf8")).toBe("{ not json");
    expect(await readdir(dirname(path))).toEqual(["settings.json"]);
  });

  it("refuses a hooks event that is not an array", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".claude", "settings.json");
    const original = await seed(path, { hooks: { Stop: { command: "x" } } });
    await expect(registerHarnesses({ root, home, nodePath, harnesses: ["claude-code"] })).rejects.toThrow("hooks.Stop");
    expect(await readFile(path, "utf8")).toBe(original);
  });
});

describe("Claude Code status-line ownership", () => {
  it("composes a command without copying it into settings, and restores all metadata", async () => {
    const statusLine = { type: "command", command: "sh ~/my-status.sh --compact", padding: 2, refreshInterval: 1000 };
    const { options, path, original } = await claudeFixture({ permissions: { allow: ["Read"] }, statusLine });
    const installedCommand = `${options.nodePath} ${join(options.root, "dist", "hosts", "claude", "claude-status.js")}`;
    await registerHarnesses(options);
    expect((await readJson(path)).statusLine).toEqual({ ...statusLine, command: installedCommand });
    expect(readClaudeStatusCommand(options.home)).toEqual(statusLine);
    expect(await backups(path)).toHaveLength(1);

    await registerHarnesses(options);
    expect(readClaudeStatusCommand(options.home)).toEqual(statusLine);
    expect(await backups(path)).toHaveLength(1);
    await unregisterHarnesses(options);
    expect(await readFile(path, "utf8")).toBe(original);
    expect(readClaudeStatusRegistration(options.home)).toBeUndefined();
  });

  it("moves from a development root to a stable managed root without nesting or duplicate hooks", async () => {
    const statusLine = { type: "command", command: "printf user-status", padding: 0 };
    const { options, path } = await claudeFixture({ statusLine });
    await registerHarnesses(options);
    const managed = { ...options, root: join(options.home, ".rerail", "current"), nodePath: "/new/node" };
    await registerHarnesses(managed);
    const settings = await readJson(path);
    expect(settings.statusLine).toMatchObject({
      ...statusLine, command: `${managed.nodePath} ${join(managed.root, "dist", "hosts", "claude", "claude-status.js")}`,
    });
    expect(readClaudeStatusCommand(options.home)).toEqual(statusLine);
    for (const groups of Object.values(settings.hooks as Record<string, { hooks: { command: string }[] }[]>)) {
      expect(groups).toHaveLength(1);
      expect(groups[0]?.hooks).toEqual([{ type: "command", command: hookLine(managed.nodePath, managed.root, "hosts/claude/claude-hook.js") }]);
    }
    await unregisterHarnesses(options);
    expect(await readJson(path)).toEqual(settings);
    await unregisterHarnesses(managed);
    expect(await readJson(path)).toEqual({ statusLine });
  });

  it("keeps a user's replacement command when unregistering", async () => {
    const { options, path } = await claudeFixture({ statusLine: { type: "command", command: "printf old" } });
    await registerHarnesses(options);
    const replacement = { type: "command", command: "printf new", padding: 3 };
    await seed(path, { ...await readJson(path), statusLine: replacement });
    await unregisterHarnesses(options);
    expect(await readJson(path)).toEqual({ statusLine: replacement });
    expect(readClaudeStatusCommand(options.home)).toBeUndefined();
  });

  it("composes a user's replacement on reinstall and keeps later metadata edits on removal", async () => {
    const { options, path } = await claudeFixture({ statusLine: { type: "command", command: "printf old" } });
    await registerHarnesses(options);
    const replacement = { type: "command", command: "printf new", padding: 3 };
    await seed(path, { ...await readJson(path), statusLine: replacement });
    await registerHarnesses(options);
    expect(readClaudeStatusCommand(options.home)).toEqual(replacement);
    const installed = await readJson(path);
    await seed(path, { ...installed, statusLine: { ...installed.statusLine as object, padding: 9 } });
    await unregisterHarnesses(options);
    expect(await readJson(path)).toEqual({ statusLine: { ...replacement, padding: 9 } });
  });

  it("does not resurrect a status line the user deleted after installation", async () => {
    const { options, path } = await claudeFixture({ statusLine: { type: "command", command: "printf old" } });
    await registerHarnesses(options);
    const installed = await readJson(path);
    delete installed.statusLine;
    await seed(path, installed);
    await unregisterHarnesses(options);
    expect(await readJson(path)).toEqual({});
    expect(readClaudeStatusCommand(options.home)).toBeUndefined();
  });

  it("rejects malformed status specifications before any harness or database write", async () => {
    const { options, path, original } = await claudeFixture({ statusLine: { type: "command", command: ["invalid"] } });
    await expect(registerHarnesses({ ...options, harnesses: ["omp", "claude-code"] })).rejects.toThrow("statusLine");
    expect(await readFile(path, "utf8")).toBe(original);
    expect(await backups(path)).toEqual([]);
    await expect(lstat(join(options.home, ".omp"))).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(claudeStatusDatabasePath(options.home))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("makes fresh readers and dry-run installs filesystem read-only", async () => {
    const { options, path } = await claudeFixture();
    expect(readClaudeStatusCommand(options.home)).toBeUndefined();
    await registerHarnesses({ ...options, dryRun: true });
    await expect(lstat(join(options.home, ".rerail"))).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(join(options.home, ".claude"))).rejects.toMatchObject({ code: "ENOENT" });
    await registerHarnesses(options);
    const registration = readClaudeStatusRegistration(options.home);
    const installed = await readFile(path, "utf8");
    await registerHarnesses({ ...options, root: join(options.home, ".rerail", "current"), dryRun: true });
    await unregisterHarnesses({ ...options, dryRun: true });
    expect(readClaudeStatusRegistration(options.home)).toEqual(registration);
    expect(await readFile(path, "utf8")).toBe(installed);
  });

  it("uses the main checkout local command above worktree-local and shared commands", async () => {
    const { options, globalPath, rootPath, cwdPath, sharedPath } = await worktreeFixture();
    const globalStatus = { type: "command", command: "printf global", padding: 1 };
    const rootStatus = { type: "command", command: "printf root", padding: 4, refreshInterval: 700 };
    const rootSettings = { statusLine: rootStatus, permissions: { allow: ["Read"] }, hooks: { Stop: [] }, custom: "kept" };
    await seed(globalPath, { statusLine: globalStatus });
    await seed(rootPath, rootSettings);
    const cwdOriginal = await seed(cwdPath, { statusLine: { type: "command", command: "printf cwd" } });
    const sharedOriginal = await seed(sharedPath, { statusLine: { type: "command", command: "printf shared" } });
    await registerHarnesses(options);
    expect(await readJson(rootPath)).toEqual({
      ...rootSettings,
      statusLine: { ...rootStatus, command: `${options.nodePath} ${join(options.root, "dist", "hosts", "claude", "claude-status.js")} ${rootPath}` },
    });
    expect(readClaudeStatusCommand(options.home, rootPath)).toEqual(rootStatus);
    expect(readClaudeStatusCommand(options.home)).toEqual(globalStatus);
    expect(await readFile(cwdPath, "utf8")).toBe(cwdOriginal);
    expect(await readFile(sharedPath, "utf8")).toBe(sharedOriginal);
    const upgraded = { ...options, root: join(options.home, ".rerail", "current"), nodePath: "/new/node", workspaceDir: options.home };
    await registerHarnesses(upgraded);
    const installed = await readJson(rootPath);
    expect(installed.statusLine).toEqual({
      ...rootStatus, command: `${upgraded.nodePath} ${join(upgraded.root, "dist", "hosts", "claude", "claude-status.js")} ${rootPath}`,
    });
    await registerHarnesses(upgraded);
    expect(await readJson(rootPath)).toEqual(installed);
    expect(readClaudeStatusCommand(options.home, rootPath)).toEqual(rootStatus);
    await unregisterHarnesses(options);
    expect(await readJson(rootPath)).toEqual(installed);
    await unregisterHarnesses(upgraded);
    expect(await readJson(rootPath)).toEqual(rootSettings);
    expect(await readJson(globalPath)).toEqual({ statusLine: globalStatus });
    expect(readClaudeStatusRegistration(options.home, rootPath)).toBeUndefined();
    expect(readClaudeStatusRegistration(options.home)).toBeUndefined();
  });

  it("locally composes shared settings without modifying them and restores inheritance", async () => {
    const { options, sharedPath, rootPath: localPath, mainSharedPath } = await worktreeFixture();
    const statusLine = { type: "command", command: "printf shared", padding: 2 };
    const sharedOriginal = await seed(sharedPath, { statusLine, permissions: { deny: ["Write"] } });
    await seed(localPath, { permissions: { allow: ["Read"] } });
    await seed(mainSharedPath, { statusLine: { type: "command", command: "printf from-A" } });
    await registerHarnesses({ ...options, workspaceDir: options.main });
    await registerHarnesses(options);
    expect(readClaudeStatusCommand(options.home, localPath, options.workspaceDir)).toEqual(statusLine);
    const changed = { ...statusLine, command: "printf changed-A" };
    await seed(mainSharedPath, { statusLine: changed });
    expect(readClaudeStatusCommand(options.home, localPath, options.main)).toEqual(changed);
    await registerHarnesses({ ...options, workspaceDir: options.main });
    expect(readClaudeStatusCommand(options.home, localPath, options.main)).toEqual(changed);
    await registerHarnesses(options);
    await unregisterHarnesses(options);
    expect(await readJson(localPath)).toEqual({ permissions: { allow: ["Read"] } });
    expect(await readFile(sharedPath, "utf8")).toBe(sharedOriginal);
  });

  it("keeps later project replacements independent from global restoration", async () => {
    const { options, rootPath: path, globalPath } = await worktreeFixture();
    const globalStatus = { type: "command", command: "printf global" };
    await seed(globalPath, { statusLine: globalStatus });
    await seed(path, { statusLine: { type: "command", command: "printf original" } });
    await registerHarnesses(options);
    const replacement = { statusLine: { type: "command", command: "printf replacement", padding: 8 }, custom: true };
    await seed(path, replacement);
    await unregisterHarnesses({ ...options, workspaceDir: options.home });
    expect(await readJson(path)).toEqual(replacement);
    expect(await readJson(globalPath)).toEqual({ statusLine: globalStatus });
    expect(readClaudeStatusRegistration(options.home, path)).toBeUndefined();
  });

  it("uses cwd-local settings on Windows and when the Git root is home", async () => {
    const { options, rootPath, cwdPath: localPath } = await worktreeFixture();
    const rootOriginal = await seed(rootPath, { statusLine: { type: "command", command: "printf root" } });
    const localStatus = { type: "command", command: "printf cwd" };
    await seed(localPath, { statusLine: localStatus });
    const windows = { ...options, platform: "win32" as const };
    await registerHarnesses(windows);
    expect(readClaudeStatusCommand(options.home, localPath)).toEqual(localStatus);
    expect(await readFile(rootPath, "utf8")).toBe(rootOriginal);
    await unregisterHarnesses(windows);
    await registerHarnesses({ ...options, home: options.main });
    expect(readClaudeStatusCommand(options.main, localPath)).toEqual(localStatus);
    expect(await readFile(rootPath, "utf8")).toBe(rootOriginal);
  });

  it("preflights project status before global writes and refuses another package's wrapper", async () => {
    const { options, rootPath: path } = await worktreeFixture();
    const foreign = await packageInstall(options, "foreign", "another-package");
    const original = await seed(path, { statusLine: { type: "command", command: `/old/node ${join(foreign, "dist", "hosts", "claude", "claude-status.js")} ${path}` } });
    await expect(registerHarnesses(options)).rejects.toThrow("registration ownership is missing");
    expect(await readFile(path, "utf8")).toBe(original);
    await expect(lstat(join(options.home, ".claude"))).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(claudeStatusDatabasePath(options.home))).rejects.toMatchObject({ code: "ENOENT" });
  });

  // Deleting an install's home leaves its generated entries behind with nothing to run; setup replaces them.
  it("replaces the status line, hooks and MCP bridge of an install that is gone, and keeps a foreign package's", async () => {
    const options = { ...await fixture(), harnesses: ["claude-code", "codex", "cursor", "omp"] as const };
    const gone = join(dirname(options.home), "deleted-home", "current");
    const foreign = await packageInstall(options, "foreign", "another-package");
    const settingsPath = join(options.home, ".claude", "settings.json");
    const statusLine = { type: "command", command: `/old/node ${join(gone, "dist", "hosts", "claude", "claude-status.js")}`, padding: 2 };
    await seed(settingsPath, {
      statusLine,
      hooks: { Stop: [{ hooks: [{ type: "command", command: hookLine("/old/node", gone, "hosts/claude/claude-hook.js") }] }] },
    });
    const foreignCodexHook = { type: "command", command: hookLine("/old/node", foreign, "hosts/codex/codex-hook.js") };
    await seed(join(options.home, ".codex", "hooks.json"), {
      hooks: { Stop: [{ hooks: [{ type: "command", command: hookLine("/old/node", gone, "hosts/codex/codex-hook.js") }, foreignCodexHook] }] },
    });
    await seed(join(options.home, ".cursor", "hooks.json"), {
      version: 1, hooks: { stop: [{ command: hookLine("/old/node", gone, "hosts/cursor/cursor-hook.js") }] },
    });
    const foreignBridge = { command: "/old/node", args: [join(foreign, "dist", "cli", "cli.js"), "mcp-bridge"] };
    const mcpPath = join(options.home, ".omp", "agent", "mcp.json");
    await seed(mcpPath, { mcpServers: { previous: { command: "/old/node", args: [join(gone, "dist", "cli", "cli.js"), "mcp-bridge"] }, foreign: foreignBridge } });

    await registerHarnesses(options);
    const settings = await readJson(settingsPath);
    expect(settings.statusLine).toEqual({ ...statusLine, command: `${options.nodePath} ${join(options.root, "dist", "hosts", "claude", "claude-status.js")}`, refreshInterval: 5 });
    expect(readClaudeStatusRegistration(options.home)).toMatchObject({ previous: null });
    expect((settings.hooks as Record<string, unknown>).Stop)
      .toEqual([{ hooks: [{ type: "command", command: hookLine(options.nodePath, options.root, "hosts/claude/claude-hook.js") }] }]);
    expect(((await readJson(join(options.home, ".codex", "hooks.json"))).hooks as Record<string, unknown>).Stop).toEqual([
      { hooks: [foreignCodexHook] },
      { hooks: [{ type: "command", command: hookLine(options.nodePath, options.root, "hosts/codex/codex-hook.js") }] },
    ]);
    expect(((await readJson(join(options.home, ".cursor", "hooks.json"))).hooks as Record<string, unknown>).stop)
      .toEqual([{ command: hookLine(options.nodePath, options.root, "hosts/cursor/cursor-hook.js") }]);
    expect(await readJson(mcpPath)).toEqual({
      mcpServers: { foreign: foreignBridge, rerail: { command: options.nodePath, args: [join(options.root, "dist", "cli", "cli.js"), "mcp-bridge"] } },
    });
  });

  it("replaces the status line and hooks of another install of this package that this home has no record of", async () => {
    const { options, path } = await claudeFixture();
    const other = await packageInstall(options, "elsewhere");
    const replaced = { type: "command", command: `/old/node ${join(other, "dist", "hosts", "claude", "claude-status.js")}`, padding: 2, refreshInterval: 3 };
    await seed(path, {
      statusLine: replaced,
      hooks: { Stop: [{ hooks: [{ type: "command", command: hookLine("/old/node", other, "hosts/claude/claude-hook.js") }] }] },
    });
    await registerHarnesses(options);
    const installed = await readJson(path);
    expect(installed.statusLine).toEqual({ ...replaced, command: `${options.nodePath} ${join(options.root, "dist", "hosts", "claude", "claude-status.js")}` });
    expect(readClaudeStatusRegistration(options.home)).toMatchObject({ previous: null });
    expect(readClaudeStatusRegistration(options.home)?.previousInherited).toBeUndefined();
    expect(readClaudeStatusCommand(options.home)).toBeUndefined();
    for (const groups of Object.values(installed.hooks as Record<string, { hooks: { command: string }[] }[]>)) {
      expect(groups).toHaveLength(1);
      expect(groups[0]?.hooks).toEqual([{ type: "command", command: hookLine(options.nodePath, options.root, "hosts/claude/claude-hook.js") }]);
    }
    await registerHarnesses(options);
    expect(await readJson(path)).toEqual(installed);
    await unregisterHarnesses(options);
    expect((await readJson(path)).statusLine).toBeUndefined();
    expect(readClaudeStatusRegistration(options.home)).toBeUndefined();
  });

  it("replaces its own status line when this home has no record of it", async () => {
    const { options, path } = await claudeFixture();
    const installedCommand = `${options.nodePath} ${join(options.root, "dist", "hosts", "claude", "claude-status.js")}`;
    await seed(path, { statusLine: { type: "command", command: installedCommand } });
    await registerHarnesses(options);
    expect((await readJson(path)).statusLine).toEqual({ type: "command", command: installedCommand, refreshInterval: 5 });
    expect(readClaudeStatusRegistration(options.home)).toMatchObject({ installedCommand, previous: null });
    await unregisterHarnesses(options);
    expect((await readJson(path)).statusLine).toBeUndefined();
  });

  it("replaces another install's status line inherited from shared project settings without inheriting it again", async () => {
    const { options, sharedPath, rootPath: localPath } = await worktreeFixture();
    const other = await packageInstall(options, "elsewhere");
    const sharedOriginal = await seed(sharedPath, {
      statusLine: { type: "command", command: `/old/node ${join(other, "dist", "hosts", "claude", "claude-status.js")} ${sharedPath}` },
    });
    await registerHarnesses(options);
    expect((await readJson(localPath)).statusLine).toEqual({
      type: "command", command: `${options.nodePath} ${join(options.root, "dist", "hosts", "claude", "claude-status.js")} ${localPath}`, refreshInterval: 5,
    });
    const registration = readClaudeStatusRegistration(options.home, localPath);
    expect(registration).toMatchObject({ previous: null });
    expect(registration?.previousInherited).toBeUndefined();
    expect(readClaudeStatusCommand(options.home, localPath, options.workspaceDir)).toBeUndefined();
    await unregisterHarnesses(options);
    expect((await readJson(localPath)).statusLine).toBeUndefined();
    expect(await readFile(sharedPath, "utf8")).toBe(sharedOriginal);
  });

  it("replaces another install's project status line and refuses one whose script belongs to a different package", async () => {
    const { options, rootPath: path } = await worktreeFixture();
    const other = await packageInstall(options, "elsewhere");
    await seed(path, { statusLine: { type: "command", command: `/old/node ${join(other, "dist", "hosts", "claude", "claude-status.js")} ${path}` } });
    await registerHarnesses(options);
    expect((await readJson(path)).statusLine).toEqual({
      type: "command", command: `${options.nodePath} ${join(options.root, "dist", "hosts", "claude", "claude-status.js")} ${path}`, refreshInterval: 5,
    });
    expect(readClaudeStatusRegistration(options.home, path)).toMatchObject({ previous: null });
    expect(readClaudeStatusCommand(options.home, path)).toBeUndefined();
    await unregisterHarnesses(options);
    expect(await readJson(path)).toEqual({});

    const foreign = await packageInstall(options, "foreign", "another-package");
    const original = await seed(path, { statusLine: { type: "command", command: `/old/node ${join(foreign, "dist", "hosts", "claude", "claude-status.js")} ${path}` } });
    await expect(registerHarnesses(options)).rejects.toThrow("registration ownership is missing");
    expect(await readFile(path, "utf8")).toBe(original);
    expect(readClaudeStatusRegistration(options.home, path)).toBeUndefined();
  });

  it("keeps scoped recovery isolated on settings write and restore failures", async () => {
    const { options, rootPath: path } = await worktreeFixture();
    const statusLine = { type: "command", command: "printf project" };
    await seed(path, { statusLine });
    await registerHarnesses(options);
    const global = readClaudeStatusRegistration(options.home);
    const project = readClaudeStatusRegistration(options.home, path)!;
    const database = openSqliteDatabase(claudeStatusDatabasePath(options.home));
    try {
      expect(database.all(sql`SELECT name FROM sqlite_master WHERE type = 'table'`)).toEqual([{ name: "harness_status_lines" }]);
      // Former installations used project-version stamps; recovery must ignore those versions.
      database.run(sql`PRAGMA user_version = 999`);
    } finally { database.$client.close(); }
    await expect(writeClaudeStatusRegistration(options.home, {
      ...project, previous: { type: "command", command: "printf replacement" },
    }, async () => { throw new Error("write failed"); })).rejects.toThrow("write failed");
    expect(readClaudeStatusRegistration(options.home, path)).toEqual(project);
    await expect(writeClaudeStatusRegistration(options.home, undefined, async () => {
      throw new Error("restore failed");
    }, path)).rejects.toThrow("restore failed");
    expect(readClaudeStatusCommand(options.home, path)).toEqual(statusLine);
    expect(readClaudeStatusRegistration(options.home)).toEqual(global);
  });

  it("does not create local settings when no native override exists", async () => {
    const { options } = await worktreeFixture();
    await registerHarnesses(options);
    await expect(lstat(join(options.main, ".claude"))).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(join(options.workspaceDir, ".claude"))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("automatically installs project status from the standard install command cwd", async () => {
    const { options, rootPath: path } = await worktreeFixture();
    const statusLine = { type: "command", command: "printf local" };
    await seed(path, { statusLine });
    vi.stubEnv("HOME", options.home);
    vi.spyOn(process, "cwd").mockReturnValue(options.workspaceDir);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const home = join(options.home, ".rerail");
    const updater = new ManagedUpdater({ home, packageRoot: options.root, packageVersion: "1.0.0" });
    await runInstallCommand(["--root", options.root, "--agents", "claude-code"], { home, packageRoot: options.root, updater });
    expect(readClaudeStatusCommand(options.home, path)).toEqual(statusLine);
  });

  it("composes an old cwd-local override when root-local has no status key", async () => {
    const { options, cwdPath: path, rootPath } = await worktreeFixture();
    const rootOriginal = await seed(rootPath, { permissions: { allow: ["Read"] } });
    const statusLine = { type: "command", command: "printf cwd", padding: 0 };
    await seed(path, { statusLine });
    await registerHarnesses(options);
    expect(readClaudeStatusCommand(options.home, path)).toEqual(statusLine);
    expect((await readJson(path)).statusLine).toMatchObject({
      ...statusLine, command: `${options.nodePath} ${join(options.root, "dist", "hosts", "claude", "claude-status.js")} ${path}`,
    });
    expect(await readFile(rootPath, "utf8")).toBe(rootOriginal);
    await unregisterHarnesses(options);
    expect(await readJson(path)).toEqual({ statusLine });
  });

  it.skipIf(process.getuid === undefined)("keeps local settings at cwd when the Git root is not owned by the user", async () => {
    const { options, rootPath, cwdPath: path } = await worktreeFixture();
    const rootOriginal = await seed(rootPath, { statusLine: { type: "command", command: "printf root" } });
    const statusLine = { type: "command", command: "printf cwd" };
    await seed(path, { statusLine });
    vi.spyOn(process, "getuid").mockReturnValue((process.getuid?.() ?? 0) + 1);
    await registerHarnesses(options);
    expect(readClaudeStatusCommand(options.home, path)).toEqual(statusLine);
    expect(await readFile(rootPath, "utf8")).toBe(rootOriginal);
  });
});

describe("Claude Code commands", () => {
  const options = ["claude-code"] as const;
  const commandFiles = RERAIL_COMMANDS.map(({ name }) => `${name}.md`).sort();

  it("writes every Rerail command with exact grants, is idempotent, and unregister deletes them", async () => {
    const { home, root, nodePath } = await fixture();
    const directory = join(home, ".claude", "commands");
    const command = `${nodePath} ${join(root, "dist", "cli", "cli.js")}`;
    const read = (name: string): Promise<string> => readFile(join(directory, `${name}.md`), "utf8");

    const first = await registerHarnesses({ root, home, nodePath, harnesses: options });
    for (const file of commandFiles) expect(change(first.changes, "claude-code", file).action).toBe("created");
    expect((await readdir(directory)).sort()).toEqual(commandFiles);
    for (const file of commandFiles) {
      // Enumerated per invocation: no command may run `daemon`, `mcp-bridge` or anything else the launcher offers.
      expect(await readFile(join(directory, file), "utf8")).not.toContain(`Bash(${command} *)`);
    }

    const overview = await read("rerail");
    expect(overview).toContain(`allowed-tools: Bash(${command} session claude-code *)\n`);
    expect(overview).toContain(`\n${command} session claude-code '\${CLAUDE_SESSION_ID}'\n`);
    const stop = await read("rerail-stop");
    expect(stop).toContain(`allowed-tools: Bash(${command} stop claude-code *)\n`);
    expect(stop).toContain(`\n${command} stop claude-code '\${CLAUDE_SESSION_ID}'\n`);
    const update = await read("rerail-update");
    expect(update).toContain(`allowed-tools: Bash(${command} update)\n`);
    expect(update).toContain(`\n${command} update\n`);
    const start = await read("rerail-start");
    expect(start).toContain(`\n${command} start --relay\n`);
    expect(start).not.toContain("recommended rule packs");
    const ui = await read("rerail-ui");
    expect(ui).toContain(`allowed-tools: Bash(${command} ui --print), Bash(${command} ui)\n`);
    expect(ui).toContain(`Under Orca (\`ORCA_TAB_ID\` set): run \`${command} ui\`.`);
    const create = await read("rerail-create");
    expect(create).toContain(
      `allowed-tools: Bash(${command} rules evidence *), Bash(${command} rules test *), Bash(${command} rules add *)\n`,
    );
    expect(create).toContain("\"$ARGUMENTS\"");
    expect(create).toContain(`Read ${root}/skills/rerail-create/SKILL.md and follow it.`);
    // The removal is never pre-approved: Claude Code asks the person before it runs.
    const rageQuit = await read("rerail-rage-quit");
    expect(rageQuit).toContain(`allowed-tools: Bash(${command} install --remove --dry-run), Bash(${command} stop)\n`);
    expect(rageQuit).toContain(`\n${command} install --remove\n`);
    const showMe = await read("rerail-show-me");
    expect(showMe).toContain(`allowed-tools: Bash(${command} show-me --json), Bash(${command} show-me --cleanup)\n`);
    expect(showMe).toContain("with your file edit or write tool");

    const second = await registerHarnesses({ root, home, nodePath, harnesses: options });
    for (const file of commandFiles) expect(change(second.changes, "claude-code", file).action).toBe("unchanged");

    const removed = await unregisterHarnesses({ root, home, nodePath, harnesses: options });
    for (const file of commandFiles) expect(change(removed.changes, "claude-code", file).action).toBe("updated");
    expect(await readdir(directory)).toEqual([]);

    const again = await unregisterHarnesses({ root, home, nodePath, harnesses: options });
    expect(change(again.changes, "claude-code", "rerail.md").action).toBe("unchanged");
  });

  it("lets /rerail-start relay each set-up question and run the printed answer command without a prompt", async () => {
    const { home, root, nodePath } = await fixture();
    const rerailHome = join(home, "rerail home");
    vi.stubEnv("RERAIL_HOME", rerailHome);
    await registerHarnesses({ root, home, nodePath, harnesses: options });
    const start = await readFile(join(home, ".claude", "commands", "rerail-start.md"), "utf8");
    const allowed = [...(start.match(/^allowed-tools: (.*)$/mu)?.[1] ?? "").matchAll(/Bash\((.*?)\)(?:, |$)/gu)].map(match => match[1]!);
    // Claude Code's `Bash(<prefix> *)` grants every command that starts with the prefix and a space.
    const granted = (line: string): boolean => allowed.some(pattern =>
      pattern.endsWith(" *") ? line.startsWith(pattern.slice(0, -1)) : line === pattern);
    // What set-up prints in the agent lane: the question, then one numbered command per answer, each run as set-up prints it.
    const context = { home: rerailHome, packageRoot: root, updater: new ManagedUpdater({ home: rerailHome, packageRoot: root, packageVersion: "1.0.0" }) };
    const answers = (): string[] => renderQuestion(readRepoQuestion({
      repoName: "acme-web",
      repoPath: "/code/acme-web",
      setupCommand: setupAnswerCommand(context, { relay: true })!,
    }), "agent").split("\n").filter(line => /^\d+\. .* setup /u.test(line)).map(line => line.replace(/^\d+\. /u, ""));

    const bareAnswers = answers();
    await mkdir(dirname(context.updater.launcherPath), { recursive: true });
    await writeFile(context.updater.launcherPath, "");
    const launcherAnswers = answers();
    expect(launcherAnswers).toHaveLength(3);
    expect(bareAnswers).toHaveLength(3);
    for (const line of [...launcherAnswers, ...bareAnswers]) expect(granted(line), line).toBe(true);
    expect(granted(`${nodePath} ${join(root, "dist", "cli", "cli.js")} start --relay`)).toBe(true);
    expect(granted(`${shellQuote(join(rerailHome, "bin", "rerail"))} daemon stop`)).toBe(false);

    expect(start).toContain("relay it to the person as written and wait for their reply");
    expect(start).toContain("run only the one command the output gives for that answer");
  });

  it("grants the answer commands set-up prints on Windows, the launcher double-quoted as cmd.exe runs it", async () => {
    const { home, root, nodePath } = await fixture();
    const rerailHome = join(home, "rerail home");
    vi.stubEnv("RERAIL_HOME", rerailHome);
    await registerHarnesses({ root, home, nodePath, platform: "win32", harnesses: options });
    const start = await readFile(join(home, ".claude", "commands", "rerail-start.md"), "utf8");
    const allowed = [...(start.match(/^allowed-tools: (.*)$/mu)?.[1] ?? "").matchAll(/Bash\((.*?)\)(?:, |$)/gu)].map(match => match[1]!);
    const updater = new ManagedUpdater({ home: rerailHome, packageRoot: root, packageVersion: "1.0.0", platform: "win32" });
    await mkdir(dirname(updater.launcherPath), { recursive: true });
    await writeFile(updater.launcherPath, "");

    const command = setupAnswerCommand({ home: rerailHome, packageRoot: root, updater }, { relay: true }, "win32")!;
    expect(command).toBe(`"${join(rerailHome, "bin", "rerail.cmd")}" setup --relay`);
    expect(allowed).toContain(`${command} *`);
  });

  it("has every coding agent's /rerail-start select the agent lane, even from a terminal", async () => {
    const { home, root, nodePath } = await fixture();
    await registerHarnesses({ root, home, nodePath, harnesses: ["claude-code", "cursor", "codex"] });
    const files = [
      join(home, ".claude", "commands", "rerail-start.md"),
      join(home, ".cursor", "commands", "rerail-start.md"),
      join(home, ".codex", "skills", "rerail-start", "SKILL.md"),
    ];
    for (const file of files) {
      const body = await readFile(file, "utf8");
      const run = body.match(/```sh\n(.*)\n```/u)?.[1];
      expect(run, file).toBe(`${nodePath} ${join(root, "dist", "cli", "cli.js")} start --relay`);
    }
  });

  it("removes this package's retired commands and leaves anyone else's", async () => {
    const { home, root, nodePath } = await fixture();
    const directory = join(home, ".claude", "commands");
    await mkdir(directory, { recursive: true });
    const ours = `Run:\n${nodePath} ${join(root, "dist", "cli", "cli.js")} session\n`;
    for (const name of ["rerail-session", "rerail-disable"]) await writeFile(join(directory, `${name}.md`), ours);
    await writeFile(join(directory, "rerail-enable.md"), "A user's own command.\n");

    const report = await registerHarnesses({ root, home, nodePath, harnesses: options });
    for (const name of ["rerail-session", "rerail-disable"]) {
      expect(change(report.changes, "claude-code", `${name}.md`).action).toBe("updated");
    }
    expect(report.changes.some(entry => entry.path.endsWith("rerail-enable.md"))).toBe(false);
    expect((await readdir(directory)).sort()).toEqual([...commandFiles, "rerail-enable.md"].sort());
  });

  it("on registration also removes the retired commands another install of this package wrote, or one that is gone", async () => {
    const install = await fixture();
    const { home, root, nodePath } = install;
    const directory = join(home, ".claude", "commands");
    await mkdir(directory, { recursive: true });
    const runs = (cli: string): string =>
      `---\nallowed-tools: Bash(${nodePath} ${cli} disable)\n---\n\n\`\`\`sh\n${nodePath} ${cli} disable\n\`\`\`\n`;
    const cli = (installRoot: string): string => join(installRoot, "dist", "cli", "cli.js");
    const elsewhere = cli(await packageInstall(install, "elsewhere"));
    const foreign = cli(await packageInstall(install, "foreign", "another-package"));
    // Each round writes retired files, then lists the ones registration must keep.
    const rounds: readonly (readonly [Record<string, string>, readonly string[]])[] = [
      [{ "rerail-disable.md": runs(elsewhere), "rerail-enable.md": runs(foreign) }, ["rerail-enable.md"]],
      [{ "rerail-disable.md": runs(`"${cli(await packageInstall(install, "old development"))}"`), "rerail-enable.md": `${runs(elsewhere)}${runs(foreign)}` }, ["rerail-enable.md"]],
      [{ "rerail-disable.md": runs(cli(join(dirname(home), "gone", "current"))), "rerail-session.md": runs(elsewhere) }, []],
    ];
    for (const [written, kept] of rounds) {
      for (const [file, text] of Object.entries(written)) await writeFile(join(directory, file), text);

      await unregisterHarnesses({ root, home, nodePath, harnesses: options });
      expect((await readdir(directory)).sort()).toEqual(Object.keys(written).sort());

      await registerHarnesses({ root, home, nodePath, harnesses: options });
      expect((await readdir(directory)).sort()).toEqual([...commandFiles, ...kept].sort());
      for (const file of kept) await rm(join(directory, file));
    }
  });

  it("rewrites a stale command that names another node, backing the old one up", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".claude", "commands", "rerail.md");
    await registerHarnesses({ root, home, nodePath, harnesses: options });
    const newNode = "/opt/node-22/bin/node";
    const report = await registerHarnesses({ root, home, nodePath: newNode, harnesses: options });
    expect(change(report.changes, "claude-code", "rerail.md").action).toBe("updated");
    expect(await readFile(path, "utf8")).toContain(`${newNode} ${join(root, "dist", "cli", "cli.js")} session claude-code`);
    expect(await backups(path)).toHaveLength(1);
  });

  it("leaves a user's own rerail.md alone on unregister", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".claude", "commands", "rerail.md");
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, "---\ndescription: my own rerail command\n---\n\nDo something else.\n");
    const report = await unregisterHarnesses({ root, home, nodePath, harnesses: options });
    expect(change(report.changes, "claude-code", "rerail.md").action).toBe("unchanged");
    expect(await readFile(path, "utf8")).toBe("---\ndescription: my own rerail command\n---\n\nDo something else.\n");
  });
});

describe("Codex hooks.json", () => {
  it("removes stale and current Rerail groups without disturbing foreign hook groups", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date("2026-09-08T14:52:09.000Z"));
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".codex", "hooks.json");
    const foreign = {
      hooks: {
        PreToolUse: [
          {
            hooks: [
              { type: "command", command: "python3 /opt/other/pre-tool.py" },
              { type: "command", command: `/opt/node/bin/node ${join(root, "dist", "hosts", "codex", "codex-hook.js")}` },
            ],
          },
        ],
        Stop: [{ hooks: [{ type: "command", command: "sh /opt/other/stop.sh" }] }],
      },
    };
    await seed(path, foreign);

    const registered = await registerHarnesses({ root, home, nodePath, harnesses: ["codex"] });
    expect(change(registered.changes, "codex", "hooks.json").action).toBe("updated");
    expect(await backups(path)).toHaveLength(1);
    const installed = await readJson(path);

    const removed = await unregisterHarnesses({ root, home, nodePath, harnesses: ["codex"] });
    expect(change(removed.changes, "codex", "hooks.json").action).toBe("updated");
    expect(await readJson(path)).toEqual({
      hooks: {
        PreToolUse: [{ hooks: [{ type: "command", command: "python3 /opt/other/pre-tool.py" }] }],
        Stop: foreign.hooks.Stop,
      },
    });
    expect(await backups(path)).toHaveLength(2);
    const snapshots = await Promise.all((await backups(path)).map((name) => readJson(join(dirname(path), name))));
    expect(snapshots).toEqual(expect.arrayContaining([foreign, installed]));
  });
});

describe("legacy package-root cutover", () => {
  it("replaces verified pre-HUD hooks across hosts without deleting lookalike or compound user commands", async () => {
    const options = await fixture();
    const legacyRoot = join(options.home, "old development", "rerail");
    const foreignRoot = join(options.home, "another-package");
    await seed(join(legacyRoot, "package.json"), { name: "@neralabs/rerail" });
    await seed(join(foreignRoot, "package.json"), { name: "another-package" });
    for (const [directory, file, executable] of [
      [".claude", "settings.json", "hosts/claude/claude-hook.js"],
      [".codex", "hooks.json", "hosts/codex/codex-hook.js"],
    ] as const) {
      const old = `${options.nodePath} "${join(legacyRoot, "dist", executable)}"`;
      const foreign = `${options.nodePath} ${join(foreignRoot, "dist", executable)}`;
      const compound = `${old} && printf user-owned`;
      await seed(join(options.home, directory, file), {
        hooks: {
          SessionStart: [{ hooks: [{ type: "command", command: old }] }],
          Custom: [{ hooks: [{ command: foreign }, { command: compound }] }],
        },
      });
    }
    await seed(join(options.home, ".cursor", "hooks.json"), {
      version: 1, hooks: { sessionStart: [{ command: `${options.nodePath} "${join(legacyRoot, "dist", "hosts", "cursor", "cursor-hook.js")}"` }] },
    });
    await seed(join(options.home, ".pi", "agent", "settings.json"), { packages: [legacyRoot, foreignRoot] });
    expect(readClaudeStatusRegistration(options.home)).toBeUndefined();
    await registerHarnesses({ ...options, harnesses: ["claude-code", "codex", "cursor", "pi"] });
    for (const [directory, file, executable] of [
      [".claude", "settings.json", "hosts/claude/claude-hook.js"],
      [".codex", "hooks.json", "hosts/codex/codex-hook.js"],
    ] as const) {
      const hooks = (await readJson(join(options.home, directory, file))).hooks as Record<string, unknown>;
      expect(hooks.SessionStart).toEqual([{ hooks: [{ type: "command", command: hookLine(options.nodePath, options.root, executable) }] }]);
      expect(hooks.Custom).toEqual([{ hooks: [
        { command: `${options.nodePath} ${join(foreignRoot, "dist", executable)}` },
        { command: `${options.nodePath} "${join(legacyRoot, "dist", executable)}" && printf user-owned` },
      ] }]);
    }
    expect(((await readJson(join(options.home, ".cursor", "hooks.json"))).hooks as Record<string, unknown>).sessionStart)
      .toEqual([{ command: hookLine(options.nodePath, options.root, "hosts/cursor/cursor-hook.js") }]);
    expect((await readJson(join(options.home, ".pi", "agent", "settings.json"))).packages).toEqual([foreignRoot, options.root]);
  });

  it("replaces what another install of this package registered: hooks, status line, extension link, MCP entry and Pi package", async () => {
    const options = { ...await fixture(), harnesses: ["claude-code", "codex", "omp", "pi"] as const };
    const bridge = await packageInstall(options, join("versions", "0.0.9"));
    const settingsPath = join(options.home, ".claude", "settings.json");
    const statusLine = { type: "command", command: `/old/node ${join(bridge, "dist", "hosts", "claude", "claude-status.js")}` };
    await seed(settingsPath, {
      statusLine,
      hooks: { Stop: [{ hooks: [{ type: "command", command: hookLine("/old/node", bridge, "hosts/claude/claude-hook.js") }] }] },
    });
    await seed(join(options.home, ".codex", "hooks.json"), {
      hooks: { Stop: [{ hooks: [{ type: "command", command: hookLine("/old/node", bridge, "hosts/codex/codex-hook.js") }] }] },
    });
    const mcpPath = join(options.home, ".omp", "agent", "mcp.json");
    await seed(mcpPath, { mcpServers: { previous: { command: "/old/node", args: [join(bridge, "dist", "cli", "cli.js"), "mcp-bridge"] } } });
    const extensions = join(options.home, ".omp", "agent", "extensions");
    await mkdir(extensions, { recursive: true });
    await symlink(bridge, join(extensions, "previous"));
    // Installs that are gone: a deleted default home, a deleted custom RERAIL_HOME (the review's /old/home/current), a
    // custom home whose `current` outlived its versions, and a cleaned npx cache.
    const deleted = join(dirname(options.home), "deleted-home", ".rerail", "current");
    const custom = join(dirname(options.home), "old", "home", "current");
    const stale = join(dirname(options.home), "stale-home", "current");
    await mkdir(dirname(stale), { recursive: true });
    await symlink(join(dirname(stale), "versions", "0.0.8"), stale);
    const npxCache = join(dirname(options.home), "npx-cache", "node_modules", "@neralabs", "rerail");
    await symlink(deleted, join(extensions, "deleted"));
    await symlink(custom, join(extensions, "custom"));
    await symlink(stale, join(extensions, "stale"));
    await seed(join(options.home, ".pi", "agent", "settings.json"), { packages: [bridge, deleted, custom, stale, npxCache] });

    await registerHarnesses(options);
    const settings = await readJson(settingsPath);
    expect(settings.statusLine).toEqual({ ...statusLine, command: `${options.nodePath} ${join(options.root, "dist", "hosts", "claude", "claude-status.js")}`, refreshInterval: 5 });
    expect((settings.hooks as Record<string, unknown>).Stop)
      .toEqual([{ hooks: [{ type: "command", command: hookLine(options.nodePath, options.root, "hosts/claude/claude-hook.js") }] }]);
    expect(((await readJson(join(options.home, ".codex", "hooks.json"))).hooks as Record<string, unknown>).Stop)
      .toEqual([{ hooks: [{ type: "command", command: hookLine(options.nodePath, options.root, "hosts/codex/codex-hook.js") }] }]);
    expect(await readJson(mcpPath)).toEqual({
      mcpServers: { rerail: { command: options.nodePath, args: [join(options.root, "dist", "cli", "cli.js"), "mcp-bridge"] } },
    });
    expect(await readdir(extensions)).toEqual(["rerail"]);
    expect((await readJson(join(options.home, ".pi", "agent", "settings.json"))).packages).toEqual([options.root]);
  });

  // A gone root shows nothing on disk, so only the path setup registers marks it as an install of this package.
  it("keeps a dangling extension link and a Pi entry that do not name a Rerail install root", async () => {
    const options = { ...await fixture(), harnesses: ["omp", "pi"] as const };
    const volume = join(dirname(options.home), "unmounted-volume");
    const tool = join(volume, "tool");
    const scoped = join(volume, "node_modules", "@acme", "rerail");
    const extensions = join(options.home, ".omp", "agent", "extensions");
    await mkdir(extensions, { recursive: true });
    await symlink(tool, join(extensions, "tool"));
    await symlink(scoped, join(extensions, "scoped"));
    const piPath = join(options.home, ".pi", "agent", "settings.json");
    const packages = ["npm:@acme/current", "git:github.com/acme/dotfiles/.rerail/current", tool, scoped];
    await seed(piPath, { packages });

    await registerHarnesses(options);
    expect((await readdir(extensions)).sort()).toEqual(["rerail", "scoped", "tool"]);
    expect(await readlink(join(extensions, "tool"))).toBe(tool);
    expect(await readlink(join(extensions, "scoped"))).toBe(scoped);
    expect((await readJson(piPath)).packages).toEqual([...packages, options.root]);
  });

  // Pi loads a package folder that holds only extensions/, or a single file, so a `current` path that still exists is
  // someone's package however little it holds.
  it("keeps a Pi package and extension link at a `current` path that still exists without a package.json or dist/", async () => {
    const options = { ...await fixture(), harnesses: ["omp", "pi"] as const };
    const folder = join(dirname(options.home), "pkgs", "mytool", "current");
    await mkdir(join(folder, "extensions"), { recursive: true });
    await writeFile(join(folder, "extensions", "hello.ts"), "export default function hello() {}\n");
    const file = join(dirname(options.home), "pkgs", "single", "current");
    await mkdir(dirname(file), { recursive: true });
    await writeFile(file, "export default function single() {}\n");
    const extensions = join(options.home, ".omp", "agent", "extensions");
    await mkdir(extensions, { recursive: true });
    await symlink(folder, join(extensions, "mytool"));
    const piPath = join(options.home, ".pi", "agent", "settings.json");
    await seed(piPath, { packages: [folder, file] });

    await registerHarnesses(options);
    expect((await readdir(extensions)).sort()).toEqual(["mytool", "rerail"]);
    expect((await readJson(piPath)).packages).toEqual([folder, file, options.root]);
  });
});

describe("Codex Rerail skills", () => {
  const skillPath = (home: string, name: string): string => join(home, ".codex", "skills", name, "SKILL.md");

  it("installs one skill per command, upgrades its launcher, and removes only the current installation", async () => {
    const options = await fixture();
    const path = skillPath(options.home, "rerail");
    await registerHarnesses({ ...options, harnesses: ["codex"] });
    for (const { name, description } of RERAIL_COMMANDS) {
      const text = await readFile(skillPath(options.home, name), "utf8");
      expect(text).toContain(`---\nname: ${name}\ndescription: ${description}.\n---\n`);
      expect(text).toContain("require_escalated");
    }
    // Codex's hooks see a shell write, not apply_patch, so its demo writes the file with one shell command.
    expect(await readFile(skillPath(options.home, "rerail-show-me"), "utf8")).toContain("with one shell command");
    expect(await readFile(path, "utf8")).toContain(`\n${options.nodePath} ${join(options.root, "dist", "cli", "cli.js")} session\n`);
    await registerHarnesses({ ...options, harnesses: ["codex"] });
    expect(await backups(path)).toEqual([]);
    const replacement = await fixture();
    await registerHarnesses({ ...options, root: replacement.root, harnesses: ["codex"] });
    expect(await readFile(path, "utf8")).toContain(`${options.nodePath} ${join(replacement.root, "dist", "cli", "cli.js")} session`);
    await unregisterHarnesses({ ...options, harnesses: ["codex"] });
    expect(await readFile(path, "utf8")).toContain(replacement.root);
    await unregisterHarnesses({ ...options, root: replacement.root, harnesses: ["codex"] });
    for (const { name } of RERAIL_COMMANDS) await expect(lstat(skillPath(options.home, name))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("keeps a user's own same-name skill on installation and removal", async () => {
    const options = await fixture();
    const path = skillPath(options.home, "rerail-ui");
    const original = "---\nname: rerail-ui\ndescription: Personal dashboard\n---\nOpen it.\n";
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, original);
    await registerHarnesses({ ...options, harnesses: ["codex"] });
    await unregisterHarnesses({ ...options, harnesses: ["codex"] });
    expect(await readFile(path, "utf8")).toBe(original);
  });

  it("replaces the earlier $rerail skill it installed and keeps anyone else's", async () => {
    const ours = await fixture();
    const path = skillPath(ours.home, "rerail");
    const earlier = `---\nname: rerail\n---\n${ours.nodePath} ${join(ours.root, "dist", "cli", "cli.js")} session\n\n${MANAGED_SKILL_MARKER}\n`;
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, earlier);
    const report = await registerHarnesses({ ...ours, harnesses: ["codex"] });
    expect(change(report.changes, "codex", "skills/rerail/SKILL.md").action).toBe("updated");
    const replaced = await readFile(path, "utf8");
    expect(replaced).not.toBe(earlier);
    expect(replaced).toContain(MANAGED_SKILL_MARKER);

    const foreign = await fixture();
    const userSkill = skillPath(foreign.home, "rerail");
    await mkdir(dirname(userSkill), { recursive: true });
    await writeFile(userSkill, "---\nname: rerail\ndescription: Personal music workflow\n---\nPlay music.\n");
    await registerHarnesses({ ...foreign, harnesses: ["codex"] });
    expect(await readFile(userSkill, "utf8")).toContain("Play music.");
  });
});

describe("Cursor hooks.json", () => {
  const foreign = {
    version: 1,
    hooks: {
      sessionStart: [{ command: "sh /opt/orca/hook.sh", timeout: 10 }],
      preToolUse: [
        { command: "python3 /opt/sidekick/hook.py" },
        { command: "/usr/bin/node /opt/nerling/dist/hooks/main.js" },
      ],
      afterFileEdit: [{ command: "sh /opt/orca/hook.sh" }],
    },
  };


  it("appends rerail entries, is idempotent, and unregister restores the original bytes", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".cursor", "hooks.json");
    const original = await seed(path, foreign);
    const command = hookLine(nodePath, root, "hosts/cursor/cursor-hook.js");

    const first = await registerHarnesses({ root, home, nodePath, harnesses: ["cursor"] });
    expect(change(first.changes, "cursor", "hooks.json").action).toBe("updated");
    const config = await readJson(path);
    expect(config.version).toBe(1);
    const hooks = config.hooks as Record<string, unknown[]>;
    expect(hooks.sessionStart).toEqual([...foreign.hooks.sessionStart, { command }]);
    expect(hooks.preToolUse).toEqual([...foreign.hooks.preToolUse, { command }]);
    expect(hooks.afterFileEdit).toEqual(foreign.hooks.afterFileEdit);
    for (const event of CURSOR_HOOK_EVENTS) expect(hooks[event]?.at(-1)).toEqual({ command });

    const second = await registerHarnesses({ root, home, nodePath, harnesses: ["cursor"] });
    expect(change(second.changes, "cursor", "hooks.json").action).toBe("unchanged");
    expect(await backups(path)).toHaveLength(1);

    await unregisterHarnesses({ root, home, nodePath, harnesses: ["cursor"] });
    expect(await readFile(path, "utf8")).toBe(original);
  });

  it("installs every Rerail command as a plain prompt file", async () => {
    const { home, root, nodePath } = await fixture();
    const directory = join(home, ".cursor", "commands");
    const command = `${nodePath} ${join(root, "dist", "cli", "cli.js")}`;

    const first = await registerHarnesses({ root, home, nodePath, harnesses: ["cursor"] });
    for (const { name, description } of RERAIL_COMMANDS) {
      expect(change(first.changes, "cursor", `${name}.md`).action).toBe("created");
      const body = await readFile(join(directory, `${name}.md`), "utf8");
      // Cursor shows a command's first line as its description, and has no front matter or grants.
      expect(body.split("\n")[0]).toBe(`${description}.`);
      expect(body).not.toContain("allowed-tools");
    }
    expect(await readFile(join(directory, "rerail.md"), "utf8")).toContain(`\n${command} session cursor\n`);
    const ui = await readFile(join(directory, "rerail-ui.md"), "utf8");
    expect(ui).toContain("Cursor with a built-in browser tool");
    expect(ui).not.toContain("Orca");

    await unregisterHarnesses({ root, home, nodePath, harnesses: ["cursor"] });
    expect(await readdir(directory)).toEqual([]);
  });

  it("creates a versioned file when absent", async () => {
    const { home, root, nodePath } = await fixture();
    await mkdir(join(home, ".cursor"));
    const report = await registerHarnesses({ root, home, nodePath });
    expect(change(report.changes, "cursor", "hooks.json").action).toBe("created");
    const config = await readJson(join(home, ".cursor", "hooks.json"));
    expect(Object.keys(config)).toEqual(["version", "hooks"]);
    expect(Object.keys(config.hooks as object)).toEqual([...CURSOR_HOOK_EVENTS]);
  });
});

describe("Pi settings.json", () => {
  it("adds the root to packages once and unregister restores the original bytes", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".pi", "agent", "settings.json");
    const original = await seed(path, { theme: "dark", defaultModel: "x", packages: ["npm:@other/pkg", "/opt/other"] });

    const first = await registerHarnesses({ root, home, nodePath, harnesses: ["pi"] });
    expect(change(first.changes, "pi", "settings.json").action).toBe("updated");
    expect(await readJson(path)).toEqual({
      theme: "dark",
      defaultModel: "x",
      packages: ["npm:@other/pkg", "/opt/other", root],
    });

    const second = await registerHarnesses({ root, home, nodePath, harnesses: ["pi"] });
    expect(change(second.changes, "pi", "settings.json").action).toBe("unchanged");
    expect(await backups(path)).toHaveLength(1);

    await unregisterHarnesses({ root, home, nodePath, harnesses: ["pi"] });
    expect(await readFile(path, "utf8")).toBe(original);
  });

  it("dedupes by resolved path and drops an emptied packages key", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".pi", "agent", "settings.json");
    await seed(path, { theme: "dark", packages: [`${root}/`] });
    const report = await registerHarnesses({ root, home, nodePath, harnesses: ["pi"] });
    expect(change(report.changes, "pi", "settings.json").action).toBe("updated");
    expect((await readJson(path)).packages).toEqual([root]);
    await unregisterHarnesses({ root, home, nodePath, harnesses: ["pi"] });
    expect(await readJson(path)).toEqual({ theme: "dark" });
  });
});

describe("OMP extensions symlink", () => {
  it("creates the symlink, is idempotent, repoints stale links, and unregisters", async () => {
    const { home, root, nodePath } = await fixture();
    const link = join(home, ".omp", "agent", "extensions", "rerail");

    const first = await registerHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(first.changes, "omp", "rerail").action).toBe("created");
    expect(await readlink(link)).toBe(root);

    const second = await registerHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(second.changes, "omp", "rerail").action).toBe("unchanged");

    const otherRoot = join(home, "other-root");
    await mkdir(otherRoot);
    const repointed = await registerHarnesses({ root: otherRoot, home, nodePath, harnesses: ["omp"] });
    expect(change(repointed.changes, "omp", "rerail").action).toBe("updated");
    expect(await readlink(link)).toBe(otherRoot);

    const foreign = await unregisterHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(foreign.changes, "omp", "rerail").action).toBe("unchanged");
    expect(await readlink(link)).toBe(otherRoot);

    const removed = await unregisterHarnesses({ root: otherRoot, home, nodePath, harnesses: ["omp"] });
    expect(change(removed.changes, "omp", "rerail").action).toBe("updated");
    await expect(lstat(link)).rejects.toMatchObject({ code: "ENOENT" });

    const again = await unregisterHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(again.changes, "omp", "rerail").action).toBe("unchanged");
  });

  it("keeps Windows junction targets intact while repointing and unregistering", async () => {
    const { home, root, nodePath } = await fixture();
    const link = join(home, ".omp", "agent", "extensions", "rerail");
    const otherRoot = join(home, "other-root");
    await mkdir(otherRoot);
    await writeFile(join(root, "root-marker"), "root");
    await writeFile(join(otherRoot, "other-marker"), "other");

    await registerHarnesses({ root, home, nodePath, platform: "win32", harnesses: ["omp"] });
    await registerHarnesses({ root: otherRoot, home, nodePath, platform: "win32", harnesses: ["omp"] });
    expect(await readFile(join(root, "root-marker"), "utf8")).toBe("root");

    await unregisterHarnesses({ root: otherRoot, home, nodePath, platform: "win32", harnesses: ["omp"] });
    expect(await readFile(join(otherRoot, "other-marker"), "utf8")).toBe("other");
    await expect(lstat(link)).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("refuses to clobber a real directory", async () => {
    const { home, root, nodePath } = await fixture();
    const link = join(home, ".omp", "agent", "extensions", "rerail");
    await mkdir(link, { recursive: true });
    await writeFile(join(link, "index.ts"), "export default () => {};");
    await expect(registerHarnesses({ root, home, nodePath, harnesses: ["omp"] })).rejects.toThrow("not a symlink");
    expect((await lstat(link)).isDirectory()).toBe(true);
    expect(await readFile(join(link, "index.ts"), "utf8")).toBe("export default () => {};");
  });

  it("declares the cwd-selected Rerail MCP bridge in user mcp.json and removes only that entry", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".omp", "agent", "mcp.json");
    await seed(path, { mcpServers: { other: { command: "other-mcp" } }, unrelated: true });

    const first = await registerHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(first.changes, "omp", "mcp.json").action).toBe("updated");
    expect(await readJson(path)).toEqual({
      mcpServers: { other: { command: "other-mcp" }, rerail: { command: nodePath, args: [join(root, "dist", "cli", "cli.js"), "mcp-bridge"] } },
      unrelated: true,
    });

    const second = await registerHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(second.changes, "omp", "mcp.json").action).toBe("unchanged");
    expect(await backups(path)).toHaveLength(1);

    const removed = await unregisterHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(removed.changes, "omp", "mcp.json").action).toBe("updated");
    expect(await readJson(path)).toEqual({ mcpServers: { other: { command: "other-mcp" } }, unrelated: true });

    const again = await unregisterHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(again.changes, "omp", "mcp.json").action).toBe("unchanged");
  });

  it("replaces another install's bridge and extension link under any name, and leaves foreign ones", async () => {
    const { home, root, nodePath } = await fixture();
    const otherInstall = join(home, "other install");
    await seed(join(otherInstall, "package.json"), { name: "@neralabs/rerail" });
    const foreign = join(home, "foreign extension");
    await seed(join(foreign, "package.json"), { name: "foreign" });
    const path = join(home, ".omp", "agent", "mcp.json");
    const otherBridge = { command: "/old/node", args: [join(otherInstall, "dist", "cli", "cli.js"), "mcp-bridge"] };
    const foreignBridge = { command: "/old/node", args: [join(foreign, "dist", "cli", "cli.js"), "mcp-bridge"] };
    await seed(path, { mcpServers: { previous: otherBridge, foreign: foreignBridge } });
    const extensions = join(home, ".omp", "agent", "extensions");
    await mkdir(extensions, { recursive: true });
    await symlink(otherInstall, join(extensions, "previous"));
    await symlink(foreign, join(extensions, "foreign"));

    await registerHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(await readJson(path)).toEqual({
      mcpServers: { foreign: foreignBridge, rerail: { command: nodePath, args: [join(root, "dist", "cli", "cli.js"), "mcp-bridge"] } },
    });
    expect((await readdir(extensions)).sort()).toEqual(["foreign", "rerail"]);

    await unregisterHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(await readJson(path)).toEqual({ mcpServers: { foreign: foreignBridge } });
    expect(await readdir(extensions)).toEqual(["foreign"]);
  });

  // An entry whose script is gone, as on an unmounted volume, is replaced only when it has the shape setup writes.
  it("replaces a gone install's bridge and keeps a gone entry that does not run node on a dist/cli/cli.js", async () => {
    const { home, root, nodePath } = await fixture();
    const gone = join(dirname(home), "deleted-home", "current");
    const unmounted = join(dirname(home), "unmounted-volume", "tool");
    const path = join(home, ".omp", "agent", "mcp.json");
    const binTool = { command: "/old/node", args: [join(unmounted, "bin", "cli.js"), "mcp-bridge"] };
    const npxTool = { command: "npx", args: [join(unmounted, "dist", "cli", "cli.js"), "mcp-bridge"] };
    await seed(path, {
      mcpServers: { previous: { command: "/old/node", args: [join(gone, "dist", "cli", "cli.js"), "mcp-bridge"] }, binTool, npxTool },
    });

    await registerHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(await readJson(path)).toEqual({
      mcpServers: { binTool, npxTool, rerail: { command: nodePath, args: [join(root, "dist", "cli", "cli.js"), "mcp-bridge"] } },
    });
  });

  it("creates the user mcp.json when absent and leaves an empty file alone on unregister", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".omp", "agent", "mcp.json");

    const created = await registerHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(created.changes, "omp", "mcp.json").action).toBe("created");
    expect(await readJson(path)).toEqual({
      mcpServers: { rerail: { command: nodePath, args: [join(root, "dist", "cli", "cli.js"), "mcp-bridge"] } },
    });

    const removed = await unregisterHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(removed.changes, "omp", "mcp.json").action).toBe("updated");
    expect(await readJson(path)).toEqual({ mcpServers: {} });
  });
});

describe("symlinked configs", () => {
  it("writes through a symlink to its target and keeps the link intact", async () => {
    const { home, root, nodePath } = await fixture();
    const link = join(home, ".claude", "settings.json");
    const target = join(home, "dotfiles", "claude.json");
    await seed(target, { theme: "dark" });
    await mkdir(dirname(link), { recursive: true });
    await symlink(target, link);

    const report = await registerHarnesses({ root, home, nodePath, harnesses: ["claude-code"] });
    expect(change(report.changes, "claude-code", "settings.json").action).toBe("updated");
    expect((await lstat(link)).isSymbolicLink()).toBe(true);
    expect(await readlink(link)).toBe(target);
    const settings = await readJson(target);
    expect(settings.theme).toBe("dark");
    expect(Object.keys(settings.hooks as object)).toContain("SessionStart");
    expect(await backups(target)).toHaveLength(1);
    expect(await readdir(dirname(link))).toEqual(["commands", "settings.json"]);
  });

  it("creates the target of a dangling symlink instead of replacing the link", async () => {
    const { home, root, nodePath } = await fixture();
    const link = join(home, ".cursor", "hooks.json");
    const target = join(home, "dotfiles", "cursor-hooks.json");
    await mkdir(dirname(link), { recursive: true });
    await symlink(target, link);

    const report = await registerHarnesses({ root, home, nodePath, harnesses: ["cursor"] });
    expect(change(report.changes, "cursor", "hooks.json").action).toBe("created");
    expect((await lstat(link)).isSymbolicLink()).toBe(true);
    expect(await readlink(link)).toBe(target);
    const config = await readJson(target);
    expect(Object.keys(config.hooks as object)).toEqual([...CURSOR_HOOK_EVENTS]);
    expect(await readJson(link)).toEqual(config);
    expect(await readdir(dirname(target))).toEqual(["cursor-hooks.json"]);
  });
});

describe("preflight", () => {
  it("writes nothing when a later harness config is malformed", async () => {
    const { home, root, nodePath } = await fixture();
    const claude = join(home, ".claude", "settings.json");
    const original = await seed(claude, { theme: "dark" });
    const cursor = join(home, ".cursor", "hooks.json");
    await mkdir(dirname(cursor), { recursive: true });
    await writeFile(cursor, "{ not json");

    await expect(
      registerHarnesses({ root, home, nodePath, harnesses: ["omp", "claude-code", "cursor"] }),
    ).rejects.toThrow(cursor);
    expect(await readFile(claude, "utf8")).toBe(original);
    expect(await readdir(dirname(claude))).toEqual(["settings.json"]);
    await expect(lstat(join(home, ".omp"))).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(claudeStatusDatabasePath(home))).rejects.toMatchObject({ code: "ENOENT" });
  });
});

describe("dry run", () => {
  it("reports every change without touching the filesystem", async () => {
    const { home, root, nodePath } = await fixture();
    const claude = join(home, ".claude", "settings.json");
    const original = await seed(claude, { theme: "dark" });
    await mkdir(join(home, ".omp", "agent", "extensions"), { recursive: true });
    await symlink(join(home, "elsewhere"), join(home, ".omp", "agent", "extensions", "rerail"));
    const project = join(home, "project");

    const report = await registerHarnesses({
      root,
      home,
      nodePath,
      harnesses: HARNESSES,
      dryRun: true,
    });
    const actions = Object.fromEntries(
      report.changes.map((entry) => [`${entry.harness}:${entry.path.slice(home.length)}`, entry.action]),
    );
    const commands = (harness: "claude-code" | "cursor", directory: string) => Object.fromEntries(
      RERAIL_COMMANDS.map(({ name }) => [`${harness}:/${directory}/commands/${name}.md`, "created"]),
    );
    expect(actions).toEqual({
      "omp:/.omp/agent/extensions/rerail": "updated",
      "omp:/.omp/agent/mcp.json": "created",
      "pi:/.pi/agent/settings.json": "created",
      "claude-code:/.claude/settings.json": "updated",
      ...commands("claude-code", ".claude"),
      "codex:/.codex/hooks.json": "created",
      ...Object.fromEntries(RERAIL_COMMANDS.map(({ name }) => [`codex:/.codex/skills/${name}/SKILL.md`, "created"])),
      "cursor:/.cursor/hooks.json": "created",
      ...commands("cursor", ".cursor"),
    });
    expect(report.changes.every((entry) => entry.detail.endsWith("(dry run)"))).toBe(true);

    expect(await readFile(claude, "utf8")).toBe(original);
    expect(await readdir(dirname(claude))).toEqual(["settings.json"]);
    await expect(lstat(join(home, ".claude", "commands"))).rejects.toMatchObject({ code: "ENOENT" });
    expect(await readlink(join(home, ".omp", "agent", "extensions", "rerail"))).toBe(join(home, "elsewhere"));
    await expect(lstat(join(home, ".omp", "agent", "mcp.json"))).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(project)).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(join(home, ".pi"))).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(join(home, ".codex"))).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(join(home, ".cursor"))).rejects.toMatchObject({ code: "ENOENT" });
  });
});
