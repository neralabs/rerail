import { execFile } from "node:child_process";
import { afterEach, describe, expect, it, vi } from "vitest";
import { lstat, mkdir, mkdtemp, readdir, readFile, readlink, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";
import { sql } from "drizzle-orm";
import { openSqliteDatabase } from "../src/database.js";
import {
  CURSOR_HOOK_EVENTS,
  HARNESSES,
  detectHarnesses,
  registerHarnesses,
  unregisterHarnesses,
  type Harness,
  type RegistrationChange,
} from "../src/harness-registration.js";
import {
  claudeStatusDatabasePath,
  readClaudeStatusCommand,
  readClaudeStatusRegistration,
  writeClaudeStatusRegistration,
} from "../src/claude-status-store.js";
import { runInstallCommand } from "../src/setup-cli.js";
import { ManagedUpdater } from "../src/updater.js";

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
  await mkdir(join(root, "skills", "rerail"), { recursive: true });
  await writeFile(join(root, "skills", "rerail", "SKILL.md"), await readFile(new URL("../skills/rerail/SKILL.md", import.meta.url)));
  const home = join(base, "home");
  await mkdir(home);
  return { home, root, nodePath: "/opt/node/bin/node" };
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
      ["codex", "created"],
      ["cursor", "skipped"],
    ]);
    expect(report.notes.some((note) => note.includes("/hooks"))).toBe(true);
    const hooks = (await readJson(join(home, ".codex", "hooks.json"))).hooks as Record<string, unknown[]>;
    expect(Object.keys(hooks)).toEqual(["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"]);
    expect(hooks.PreToolUse).toEqual([
      { hooks: [{ type: "command", command: hookLine(nodePath, root, "codex-hook.js") }] },
    ]);
    expect(hooks.PostToolUse).toEqual([
      { matcher: "", hooks: [{ type: "command", command: hookLine(nodePath, root, "codex-hook.js") }] },
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
    const hookPath = join(root, "dist", "codex-hook.js");
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
    const hookPath = join(root, "dist", "codex-hook.js");
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
    const command = hookLine(nodePath, root, "claude-hook.js");

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
    const stale = `${nodePath} ${join(root, "dist", "claude-hook.js")}`;
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
      { hooks: [{ type: "command", command: hookLine(newNode, root, "claude-hook.js") }] },
    ]);
  });

  it("registers Windows hooks through the exec-form launcher and replaces the cmd wrapper Git Bash mangles", async () => {
    const { home, root } = await fixture();
    const nodePath = String.raw`C:\Program Files\nodejs\node.exe`;
    const hookPath = join(root, "dist", "claude-hook.js");
    const launcherPath = join(root, "dist", "hook-launcher.js");
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
    const installedCommand = `${options.nodePath} ${join(options.root, "dist", "claude-status.js")}`;
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
      ...statusLine, command: `${managed.nodePath} ${join(managed.root, "dist", "claude-status.js")}`,
    });
    expect(readClaudeStatusCommand(options.home)).toEqual(statusLine);
    for (const groups of Object.values(settings.hooks as Record<string, { hooks: { command: string }[] }[]>)) {
      expect(groups).toHaveLength(1);
      expect(groups[0]?.hooks).toEqual([{ type: "command", command: hookLine(managed.nodePath, managed.root, "claude-hook.js") }]);
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
      statusLine: { ...rootStatus, command: `${options.nodePath} ${join(options.root, "dist", "claude-status.js")} ${rootPath}` },
    });
    expect(readClaudeStatusCommand(options.home, rootPath)).toEqual(rootStatus);
    expect(readClaudeStatusCommand(options.home)).toEqual(globalStatus);
    expect(await readFile(cwdPath, "utf8")).toBe(cwdOriginal);
    expect(await readFile(sharedPath, "utf8")).toBe(sharedOriginal);
    const upgraded = { ...options, root: join(options.home, ".rerail", "current"), nodePath: "/new/node", workspaceDir: options.home };
    await registerHarnesses(upgraded);
    const installed = await readJson(rootPath);
    expect(installed.statusLine).toEqual({
      ...rootStatus, command: `${upgraded.nodePath} ${join(upgraded.root, "dist", "claude-status.js")} ${rootPath}`,
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

  it("preflights project status before global writes and refuses orphaned wrappers", async () => {
    const { options, rootPath: path } = await worktreeFixture();
    const original = await seed(path, { statusLine: { type: "command", command: `/old/node /retired/rerail/dist/claude-status.js ${path}` } });
    await expect(registerHarnesses(options)).rejects.toThrow();
    expect(await readFile(path, "utf8")).toBe(original);
    await expect(lstat(join(options.home, ".claude"))).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(claudeStatusDatabasePath(options.home))).rejects.toMatchObject({ code: "ENOENT" });
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
    await runInstallCommand(["--root", options.root, "--harness", "claude-code"], { home, packageRoot: options.root, updater });
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
      ...statusLine, command: `${options.nodePath} ${join(options.root, "dist", "claude-status.js")} ${path}`,
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

describe("Claude Code commands/rerail.md", () => {
  const options = ["claude-code"] as const;

  it("writes the launcher command, is idempotent, and unregister deletes it", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".claude", "commands", "rerail.md");
    const command = `${nodePath} ${join(root, "dist", "cli.js")}`;

    const first = await registerHarnesses({ root, home, nodePath, harnesses: options });
    expect(change(first.changes, "claude-code", "rerail.md").action).toBe("created");
    const text = await readFile(path, "utf8");
    // Enumerated per subcommand: the body must never be able to run `start`, `stop`, or `mcp`.
    expect(text).toContain(
      `allowed-tools: Bash(${command} status), Bash(${command} session *), Bash(${command} disable), Bash(${command} enable), Bash(${command} update *), Bash(${command} reset), Bash(${command} scan *), Bash(${command} settings *)`,
    );
    expect(text).not.toContain(`Bash(${command} *)`);
    expect(text).toContain(`\n${command} $ARGUMENTS\n`);
    expect(text).toContain(`${command} session claude-code '\${CLAUDE_SESSION_ID}'`);
    for (const action of ["disable", "enable"]) {
      const commandPath = join(home, ".claude", "commands", `rerail-${action}.md`);
      expect(change(first.changes, "claude-code", `rerail-${action}.md`).action).toBe("created");
      expect(await readFile(commandPath, "utf8")).toContain(`${command} ${action}`);
    }
    expect(change(first.changes, "claude-code", "rerail-ui.md").action).toBe("created");
    const ui = await readFile(join(home, ".claude", "commands", "rerail-ui.md"), "utf8");
    expect(ui).toContain(`allowed-tools: Bash(${command} ui --print), Bash(${command} ui)\n`);
    expect(ui).toContain(`\n${command} ui --print\n`);
    expect(ui).toContain(`Under Orca (\`ORCA_TAB_ID\` set): run \`${command} ui\`.`);
    expect(ui).toContain(`desktop browser: \`${command} ui\`.`);
    expect(ui).not.toContain("orca tab create");
    expect(change(first.changes, "claude-code", "rerail-session.md").action).toBe("created");
    const session = await readFile(join(home, ".claude", "commands", "rerail-session.md"), "utf8");
    expect(session).toContain(`allowed-tools: Bash(${command} session claude-code *)\n`);
    expect(session).toContain(`\n${command} session claude-code '\${CLAUDE_SESSION_ID}'\n`);
    expect(session).toContain(`Under Orca (\`ORCA_TAB_ID\` set): run \`${command} session claude-code '\${CLAUDE_SESSION_ID}' --open\`.`);
    expect(session).toContain(`desktop browser: \`${command} session claude-code '\${CLAUDE_SESSION_ID}' --open\`.`);
    expect(session).not.toContain("orca tab create");

    const second = await registerHarnesses({ root, home, nodePath, harnesses: options });
    expect(change(second.changes, "claude-code", "rerail.md").action).toBe("unchanged");
    expect(await readdir(dirname(path))).toEqual(["rerail-disable.md", "rerail-enable.md", "rerail-session.md", "rerail-ui.md", "rerail.md"]);

    const removed = await unregisterHarnesses({ root, home, nodePath, harnesses: options });
    expect(change(removed.changes, "claude-code", "rerail.md").action).toBe("updated");
    await expect(lstat(path)).rejects.toMatchObject({ code: "ENOENT" });
    for (const name of ["disable", "enable", "ui", "session"]) {
      await expect(lstat(join(home, ".claude", "commands", `rerail-${name}.md`))).rejects.toMatchObject({ code: "ENOENT" });
    }

    const again = await unregisterHarnesses({ root, home, nodePath, harnesses: options });
    expect(change(again.changes, "claude-code", "rerail.md").action).toBe("unchanged");
  });

  it("rewrites a stale command that names another node, backing the old one up", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".claude", "commands", "rerail.md");
    await registerHarnesses({ root, home, nodePath, harnesses: options });
    const newNode = "/opt/node-22/bin/node";
    const report = await registerHarnesses({ root, home, nodePath: newNode, harnesses: options });
    expect(change(report.changes, "claude-code", "rerail.md").action).toBe("updated");
    expect(await readFile(path, "utf8")).toContain(`${newNode} ${join(root, "dist", "cli.js")} $ARGUMENTS`);
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
              { type: "command", command: `/opt/node/bin/node ${join(root, "dist", "codex-hook.js")}` },
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
      [".claude", "settings.json", "claude-hook.js"],
      [".codex", "hooks.json", "codex-hook.js"],
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
      version: 1, hooks: { sessionStart: [{ command: `${options.nodePath} "${join(legacyRoot, "dist", "cursor-hook.js")}"` }] },
    });
    await seed(join(options.home, ".pi", "agent", "settings.json"), { packages: [legacyRoot, foreignRoot] });
    expect(readClaudeStatusRegistration(options.home)).toBeUndefined();
    await registerHarnesses({ ...options, harnesses: ["claude-code", "codex", "cursor", "pi"] });
    for (const [directory, file, executable] of [
      [".claude", "settings.json", "claude-hook.js"],
      [".codex", "hooks.json", "codex-hook.js"],
    ] as const) {
      const hooks = (await readJson(join(options.home, directory, file))).hooks as Record<string, unknown>;
      expect(hooks.SessionStart).toEqual([{ hooks: [{ type: "command", command: hookLine(options.nodePath, options.root, executable) }] }]);
      expect(hooks.Custom).toEqual([{ hooks: [
        { command: `${options.nodePath} ${join(foreignRoot, "dist", executable)}` },
        { command: `${options.nodePath} "${join(legacyRoot, "dist", executable)}" && printf user-owned` },
      ] }]);
    }
    expect(((await readJson(join(options.home, ".cursor", "hooks.json"))).hooks as Record<string, unknown>).sessionStart)
      .toEqual([{ command: hookLine(options.nodePath, options.root, "cursor-hook.js") }]);
    expect((await readJson(join(options.home, ".pi", "agent", "settings.json"))).packages).toEqual([foreignRoot, options.root]);
  });
});

describe("Codex Rerail skill", () => {
  it("installs the canonical skill, upgrades its launcher, and removes only the current installation", async () => {
    const options = await fixture();
    const path = join(options.home, ".codex", "skills", "rerail", "SKILL.md");
    await registerHarnesses({ ...options, harnesses: ["codex"] });
    const first = await readFile(path, "utf8");
    expect(first).toContain(`${options.nodePath} ${join(options.root, "dist", "cli.js")} session`);
    await registerHarnesses({ ...options, harnesses: ["codex"] });
    expect(await backups(path)).toEqual([]);
    const replacement = await fixture();
    await registerHarnesses({ ...options, root: replacement.root, harnesses: ["codex"] });
    expect(await readFile(path, "utf8")).toContain(`${options.nodePath} ${join(replacement.root, "dist", "cli.js")} session`);
    await unregisterHarnesses({ ...options, harnesses: ["codex"] });
    expect(await readFile(path, "utf8")).toContain(replacement.root);
    await unregisterHarnesses({ ...options, root: replacement.root, harnesses: ["codex"] });
    await expect(lstat(path)).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("keeps a user's own same-name skill on installation and removal", async () => {
    const options = await fixture();
    const path = join(options.home, ".codex", "skills", "rerail", "SKILL.md");
    const original = "---\nname: rerail\ndescription: Personal music workflow\n---\nPlay music.\n";
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, original);
    await registerHarnesses({ ...options, harnesses: ["codex"] });
    await unregisterHarnesses({ ...options, harnesses: ["codex"] });
    expect(await readFile(path, "utf8")).toBe(original);
  });

  it("does not write hooks when the packaged skill cannot be read", async () => {
    const options = await fixture();
    await rm(join(options.root, "skills"), { recursive: true });
    await expect(registerHarnesses({ ...options, harnesses: ["codex"] })).rejects.toMatchObject({ code: "ENOENT" });
    await expect(lstat(join(options.home, ".codex"))).rejects.toMatchObject({ code: "ENOENT" });
  });

  it("reads the packaged skill from templateRoot while naming root, so a dry run can plan against a link not yet created", async () => {
    const options = await fixture();
    const current = join(options.home, ".rerail", "current");
    const path = join(options.home, ".codex", "skills", "rerail", "SKILL.md");
    const planned = await registerHarnesses({ ...options, root: current, templateRoot: options.root, harnesses: ["codex"], dryRun: true });
    expect(change(planned.changes, "codex", "SKILL.md")).toMatchObject({ action: "created", detail: expect.stringContaining("dry run") });
    await expect(lstat(path)).rejects.toMatchObject({ code: "ENOENT" });
    await registerHarnesses({ ...options, root: current, templateRoot: options.root, harnesses: ["codex"] });
    const installed = await readFile(path, "utf8");
    expect(installed).toContain(`${options.nodePath} ${join(current, "dist", "cli.js")} session`);
    expect(installed).not.toContain(options.root);
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
    const command = hookLine(nodePath, root, "cursor-hook.js");

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

  it("installs exact checkout control and rules page slash commands", async () => {
    const { home, root, nodePath } = await fixture();
    const directory = join(home, ".cursor", "commands");
    const command = `${nodePath} ${join(root, "dist", "cli.js")}`;

    const first = await registerHarnesses({ root, home, nodePath, harnesses: ["cursor"] });
    for (const action of ["disable", "enable"]) {
      const path = join(directory, `rerail-${action}.md`);
      expect(change(first.changes, "cursor", `rerail-${action}.md`).action).toBe("created");
      expect(await readFile(path, "utf8")).toContain(`${command} ${action}`);
    }
    expect(change(first.changes, "cursor", "rerail-ui.md").action).toBe("created");
    const ui = await readFile(join(directory, "rerail-ui.md"), "utf8");
    expect(ui).not.toContain("allowed-tools");
    expect(ui).toContain(`\n${command} ui --print\n`);
    expect(ui).toContain("Cursor with a built-in browser tool");
    expect(ui).toContain(`desktop browser: \`${command} ui\`.`);
    expect(ui).not.toContain("Orca");
    expect(await readdir(directory)).toEqual(["rerail-disable.md", "rerail-enable.md", "rerail-ui.md"]);

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
      mcpServers: { other: { command: "other-mcp" }, rerail: { command: nodePath, args: [join(root, "dist", "cli.js"), "mcp-bridge"] } },
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

  it("creates the user mcp.json when absent and leaves an empty file alone on unregister", async () => {
    const { home, root, nodePath } = await fixture();
    const path = join(home, ".omp", "agent", "mcp.json");

    const created = await registerHarnesses({ root, home, nodePath, harnesses: ["omp"] });
    expect(change(created.changes, "omp", "mcp.json").action).toBe("created");
    expect(await readJson(path)).toEqual({
      mcpServers: { rerail: { command: nodePath, args: [join(root, "dist", "cli.js"), "mcp-bridge"] } },
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
      projectDir: project,
      dryRun: true,
    });
    const actions = Object.fromEntries(
      report.changes.map((entry) => [`${entry.harness}:${entry.path.slice(home.length)}`, entry.action]),
    );
    expect(actions).toEqual({
      "omp:/.omp/agent/extensions/rerail": "updated",
      "omp:/.omp/agent/mcp.json": "created",
      "omp:/project/.omp/cosine-rules.json": "created",
      "pi:/.pi/agent/settings.json": "created",
      "pi:/project/.pi/cosine-rules.json": "created",
      "claude-code:/.claude/settings.json": "updated",
      "claude-code:/.claude/commands/rerail.md": "created",
      "claude-code:/.claude/commands/rerail-disable.md": "created",
      "claude-code:/.claude/commands/rerail-enable.md": "created",
      "claude-code:/.claude/commands/rerail-ui.md": "created",
      "claude-code:/.claude/commands/rerail-session.md": "created",
      "claude-code:/project/.claude/cosine-rules.json": "created",
      "codex:/.codex/hooks.json": "created",
      "codex:/.codex/skills/rerail/SKILL.md": "created",
      "codex:/project/.codex/cosine-rules.json": "created",
      "cursor:/.cursor/hooks.json": "created",
      "cursor:/.cursor/commands/rerail-disable.md": "created",
      "cursor:/.cursor/commands/rerail-enable.md": "created",
      "cursor:/.cursor/commands/rerail-ui.md": "created",
      "cursor:/project/.cursor/cosine-rules.json": "created",
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

describe("project rules", () => {
  it("seeds cosine-rules.json once per selected harness and never overwrites", async () => {
    const { home, root, nodePath } = await fixture();
    const project = join(home, "project");
    const first = await registerHarnesses({
      root,
      home,
      nodePath,
      harnesses: ["claude-code", "cursor"],
      projectDir: project,
    });
    expect(change(first.changes, "claude-code", "cosine-rules.json").action).toBe("created");
    expect(change(first.changes, "cursor", "cosine-rules.json").action).toBe("created");
    expect(first.changes.filter((entry) => entry.path.endsWith("cosine-rules.json"))).toHaveLength(2);
    expect(await readJson(join(project, ".claude", "cosine-rules.json"))).toEqual({ version: 1, rules: [] });
    expect(await readJson(join(project, ".cursor", "cosine-rules.json"))).toEqual({ version: 1, rules: [] });
    await expect(lstat(join(project, ".codex"))).rejects.toMatchObject({ code: "ENOENT" });

    const custom = '{"version":1,"rules":[{"id":"keep-me"}]}';
    await writeFile(join(project, ".claude", "cosine-rules.json"), custom);
    const second = await registerHarnesses({
      root,
      home,
      nodePath,
      harnesses: ["claude-code", "cursor"],
      projectDir: project,
    });
    expect(change(second.changes, "claude-code", "cosine-rules.json").action).toBe("unchanged");
    expect(change(second.changes, "cursor", "cosine-rules.json").action).toBe("unchanged");
    expect(await readFile(join(project, ".claude", "cosine-rules.json"), "utf8")).toBe(custom);
    expect(await readdir(join(project, ".claude"))).toEqual(["cosine-rules.json"]);

    const removed = await unregisterHarnesses({
      root,
      home,
      nodePath,
      harnesses: ["claude-code", "cursor"],
      projectDir: project,
    });
    expect(removed.changes.some((entry) => entry.path.endsWith("cosine-rules.json"))).toBe(false);
    expect(await readFile(join(project, ".claude", "cosine-rules.json"), "utf8")).toBe(custom);
  });
});
