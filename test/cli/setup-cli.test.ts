import { afterEach, describe, expect, it, vi } from "vitest";
const daemonStop = vi.hoisted(() => ({ beforeStopped: undefined as undefined | ((home: string) => Promise<void>) }));
vi.mock("../../src/runtime/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/runtime/daemon-control.js")>("../../src/runtime/daemon-control.js");
  return {
    ...actual,
    // A running daemon's exit closes its last connection; the test stands in for that close.
    stopDaemon: async (home: string) => {
      if (daemonStop.beforeStopped === undefined) return await actual.stopDaemon(home);
      await daemonStop.beforeStopped(home);
      return "stopped" as const;
    },
  };
});
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readdir, readFile, readlink, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runInstallCommand, type SetupContext } from "../../src/cli/setup-cli.js";
import { ManagedUpdater } from "../../src/setup/index.js";

const temporaryDirectories: string[] = [];
const previousHome = process.env.HOME;

afterEach(async () => {
  daemonStop.beforeStopped = undefined;
  vi.restoreAllMocks();
  if (previousHome === undefined) delete process.env.HOME;
  else process.env.HOME = previousHome;
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

interface Fixture {
  context: SetupContext;
  /** The host config home `install` writes into; `process.env.HOME` points here for the test. */
  hostConfigHome: string;
  packageRoot: string;
}

/** A checkout-shaped package root plus an empty Rerail home, with Claude Code the only detected host. */
async function fixture(): Promise<Fixture> {
  const base = await mkdtemp(join(tmpdir(), "rerail-setup-cli-"));
  temporaryDirectories.push(base);
  const packageRoot = join(base, "checkout", "rerail");
  await mkdir(join(packageRoot, "dist"), { recursive: true });
  await writeFile(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "1.0.0" }));
  const hostConfigHome = join(base, "home");
  await mkdir(join(hostConfigHome, ".claude"), { recursive: true });
  process.env.HOME = hostConfigHome;
  vi.spyOn(process, "cwd").mockReturnValue(base);
  const home = join(base, "rerail-home");
  return {
    context: { home, packageRoot, updater: new ManagedUpdater({ home, packageRoot, packageVersion: "1.0.0" }) },
    hostConfigHome,
    packageRoot,
  };
}

describe("rerail install --root", () => {
  it("points current at the checkout and writes the launcher shim", async () => {
    const { context, packageRoot } = await fixture();
    const { updater } = context;

    expect(await runInstallCommand(["--root", packageRoot, "--agents", "claude-code"], context)).toBe(0);

    expect(await readlink(updater.currentLink)).toBe(packageRoot);
    expect((await stat(updater.launcherPath)).mode & 0o111).toBe(0o111);

    // The checkout is recorded as current, but stays unmanaged: no rollback target is invented for it.
    const status = await updater.status();
    expect(status.current).toEqual({ version: "1.0.0", packageRoot });
    expect(status.previous).toBeUndefined();
  });

  it("registers hooks through current, and --remove finds them there", async () => {
    const { context, hostConfigHome, packageRoot } = await fixture();
    const settingsPath = join(hostConfigHome, ".claude", "settings.json");
    const hookPath = join(context.updater.currentLink, "dist", "hosts", "claude", "claude-hook.js");

    await runInstallCommand(["--root", packageRoot, "--agents", "claude-code"], context);

    const settings = JSON.parse(await readFile(settingsPath, "utf8")) as {
      hooks: Record<string, Array<{ hooks: Array<{ command: string }> }>>;
    };
    // One stable path, not the checkout's: retiring the workspace cannot orphan the hooks.
    for (const groups of Object.values(settings.hooks)) {
      for (const group of groups) for (const hook of group.hooks) expect(hook.command).toContain(hookPath);
    }
    expect(await readFile(join(hostConfigHome, ".claude", "commands", "rerail.md"), "utf8")).toContain(
      join(context.updater.currentLink, "dist", "cli", "cli.js"),
    );

    await runInstallCommand(["--root", packageRoot, "--agents", "claude-code", "--remove"], context);

    expect(JSON.parse(await readFile(settingsPath, "utf8"))).toEqual({});
    expect(existsSync(join(hostConfigHome, ".claude", "commands", "rerail.md"))).toBe(false);
  });

  it("writes nothing under the Rerail home for a dry run", async () => {
    const { context, packageRoot } = await fixture();
    const { updater } = context;

    expect(await runInstallCommand(["--root", packageRoot, "--dry-run"], context)).toBe(0);

    expect(existsSync(updater.currentLink)).toBe(false);
    expect(existsSync(updater.launcherPath)).toBe(false);
    expect(existsSync(context.home)).toBe(false);
  });

  it("backs up rules and history on uninstall, and keeps them when only one coding agent is removed", async () => {
    const { context, hostConfigHome, packageRoot } = await fixture();
    await runInstallCommand(["--root", packageRoot, "--agents", "claude-code"], context);
    const files = { "rerail.sqlite": "rules and history", "rerail.sqlite-wal": "wal" };
    for (const [name, text] of Object.entries(files)) await writeFile(join(context.home, name), text);

    await runInstallCommand(["--root", packageRoot, "--agents", "claude-code", "--remove"], context);
    for (const name of Object.keys(files)) expect(existsSync(join(context.home, name))).toBe(true);

    await runInstallCommand(["--root", packageRoot, "--remove"], context);
    const backups = (await readdir(hostConfigHome)).filter(name => name.startsWith(".rerail-clean-"));
    expect(backups).toEqual([expect.stringMatching(/^\.rerail-clean-\d{8}$/u)]);
    for (const [name, text] of Object.entries(files)) {
      expect(existsSync(join(context.home, name))).toBe(false);
      expect(await readFile(join(hostConfigHome, backups[0]!, "rerail-home", name), "utf8")).toBe(text);
    }

    // A second uninstall the same day never overwrites the first backup.
    await writeFile(join(context.home, "rerail.sqlite"), "later");
    await runInstallCommand(["--root", packageRoot, "--remove"], context);
    const later = (await readdir(hostConfigHome)).filter(name => /^\.rerail-clean-\d{8}-\d{6}$/u.test(name));
    expect(later).toHaveLength(1);
    expect(await readFile(join(hostConfigHome, later[0]!, "rerail-home", "rerail.sqlite"), "utf8")).toBe("later");
  });

  it("backs up every file a running daemon leaves once it has stopped and checkpointed its write-ahead log", async () => {
    const { context, hostConfigHome, packageRoot } = await fixture();
    await runInstallCommand(["--root", packageRoot, "--agents", "claude-code"], context);
    const files = { "rerail.sqlite": "rules and history", "rerail.sqlite-wal": "wal", "rerail.sqlite-shm": "shm" };
    for (const [name, text] of Object.entries(files)) await writeFile(join(context.home, name), text);
    // SQLite deletes the sidecars when the daemon's last connection closes.
    daemonStop.beforeStopped = async home => {
      await rm(join(home, "rerail.sqlite-wal"));
      await rm(join(home, "rerail.sqlite-shm"));
    };

    expect(await runInstallCommand(["--root", packageRoot, "--remove"], context)).toBe(0);

    const [backup] = (await readdir(hostConfigHome)).filter(name => name.startsWith(".rerail-clean-"));
    expect(await readdir(join(hostConfigHome, backup!, "rerail-home"))).toEqual(["rerail.sqlite"]);
    expect((await readdir(context.home)).filter(name => name.includes(".sqlite"))).toEqual([]);
  });

  it("keeps rules and history when another checkout registered with --root is removed", async () => {
    const { context, hostConfigHome, packageRoot } = await fixture();
    await runInstallCommand(["--root", packageRoot, "--agents", "claude-code"], context);
    await writeFile(join(context.home, "rerail.sqlite"), "live database");
    // The undo names that checkout, not the managed install `current` points at.
    const checkoutRoot = join(hostConfigHome, "..", "checkout");
    await mkdir(join(checkoutRoot, "dist"), { recursive: true });

    expect(await runInstallCommand(["--root", checkoutRoot, "--remove"], context)).toBe(0);

    expect(await readFile(join(context.home, "rerail.sqlite"), "utf8")).toBe("live database");
    expect((await readdir(hostConfigHome)).filter(name => name.startsWith(".rerail-clean-"))).toEqual([]);
  });
});

describe("rerail install --refresh", () => {
  function captureOutput(): { stdout: () => string; stderr: () => string } {
    const out: string[] = [];
    const err: string[] = [];
    vi.spyOn(process.stdout, "write").mockImplementation(chunk => (out.push(String(chunk)), true));
    vi.spyOn(process.stderr, "write").mockImplementation(chunk => (err.push(String(chunk)), true));
    return { stdout: () => out.join(""), stderr: () => err.join("") };
  }

  it("brings back a command a connected agent lacks and leaves an agent it never connected alone", async () => {
    const { context, hostConfigHome, packageRoot } = await fixture();
    await runInstallCommand(["--root", packageRoot, "--agents", "claude-code"], context);
    const showMe = join(hostConfigHome, ".claude", "commands", "rerail-show-me.md");
    await rm(showMe);
    await mkdir(join(hostConfigHome, ".cursor"));
    const output = captureOutput();

    expect(await runInstallCommand(["--refresh"], context)).toBe(0);

    expect(await readFile(showMe, "utf8")).toContain(join(context.updater.currentLink, "dist", "cli", "cli.js"));
    expect(await readdir(join(hostConfigHome, ".cursor"))).toEqual([]);
    expect(output.stdout()).toContain("Claude Code");
    expect(output.stdout()).toContain(showMe);
    // Only what changed is listed, not every file already in place.
    expect(output.stdout()).not.toContain(join(hostConfigHome, ".claude", "commands", "rerail.md"));
  });

  it("says so and writes nothing when no coding agent is connected", async () => {
    const { context, hostConfigHome } = await fixture();
    const output = captureOutput();
    expect(await runInstallCommand(["--refresh"], context)).toBe(0);
    expect(output.stdout()).toContain("No coding agents are connected");
    expect(await readdir(join(hostConfigHome, ".claude"))).toEqual([]);
  });

  it("does not claim nothing is connected when an agent's configuration could not be read to tell", async () => {
    const { context, hostConfigHome } = await fixture();
    await mkdir(join(hostConfigHome, ".cursor"));
    await writeFile(join(hostConfigHome, ".cursor", "hooks.json"), "{ not json");
    const output = captureOutput();
    expect(await runInstallCommand(["--refresh"], context)).toBe(0);
    expect(output.stdout()).not.toContain("No coding agents are connected");
    expect(output.stdout()).toContain("left Cursor alone");
  });

  it("fails naming a connected agent it cannot register again, after refreshing the others", async () => {
    const { context, hostConfigHome, packageRoot } = await fixture();
    await mkdir(join(hostConfigHome, ".codex"));
    await runInstallCommand(["--root", packageRoot, "--agents", "claude-code,codex"], context);
    await rm(join(hostConfigHome, ".claude", "commands", "rerail-show-me.md"));
    const codexHooks = join(hostConfigHome, ".codex", "hooks.json");
    const config = JSON.parse(await readFile(codexHooks, "utf8")) as { hooks: Record<string, unknown> };
    config.hooks.SessionStart = "not a list";
    await writeFile(codexHooks, JSON.stringify(config));
    const output = captureOutput();

    expect(await runInstallCommand(["--refresh"], context)).toBe(1);

    expect(existsSync(join(hostConfigHome, ".claude", "commands", "rerail-show-me.md"))).toBe(true);
    expect(output.stderr()).toContain("Codex");
    expect(output.stderr()).toContain(codexHooks);
    expect(output.stderr()).toContain("rerail install --refresh");
  });

  it("succeeds, naming it, when an agent's configuration cannot be read to tell whether it is connected", async () => {
    const { context, hostConfigHome, packageRoot } = await fixture();
    await runInstallCommand(["--root", packageRoot, "--agents", "claude-code"], context);
    await mkdir(join(hostConfigHome, ".cursor"));
    const cursorHooks = join(hostConfigHome, ".cursor", "hooks.json");
    await writeFile(cursorHooks, "{ not json");
    const output = captureOutput();

    expect(await runInstallCommand(["--refresh"], context)).toBe(0);

    expect(output.stdout()).toContain("left Cursor alone");
    expect(output.stdout()).toContain(cursorHooks);
    expect(output.stderr()).toBe("");
    expect(await readFile(cursorHooks, "utf8")).toBe("{ not json");
  });

  it.each([[["--agents", "claude-code"]], [["--root", "."]], [["--remove"]], [["--dry-run"]]])("refuses --refresh with %j", async flag => {
    const { context } = await fixture();
    await expect(runInstallCommand(["--refresh", ...flag], context)).rejects.toThrow("--refresh");
  });
});
