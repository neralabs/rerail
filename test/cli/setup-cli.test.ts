import { afterEach, describe, expect, it, vi } from "vitest";
const daemonStop = vi.hoisted(() => ({ beforeStopped: undefined as undefined | ((home: string) => Promise<void>) }));
vi.mock("../src/daemon-control.js", async () => {
  const actual = await vi.importActual<typeof import("../src/daemon-control.js")>("../src/daemon-control.js");
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
import { runInstallCommand, runSetupCommand, type SetupContext } from "../src/setup-cli.js";
import { ManagedUpdater } from "../src/updater.js";

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

    expect(await runInstallCommand(["--root", packageRoot, "--harness", "claude-code"], context)).toBe(0);

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
    const hookPath = join(context.updater.currentLink, "dist", "claude-hook.js");

    await runInstallCommand(["--root", packageRoot, "--harness", "claude-code"], context);

    const settings = JSON.parse(await readFile(settingsPath, "utf8")) as {
      hooks: Record<string, Array<{ hooks: Array<{ command: string }> }>>;
    };
    // One stable path, not the checkout's: retiring the workspace cannot orphan the hooks.
    for (const groups of Object.values(settings.hooks)) {
      for (const group of groups) for (const hook of group.hooks) expect(hook.command).toContain(hookPath);
    }
    expect(await readFile(join(hostConfigHome, ".claude", "commands", "rerail.md"), "utf8")).toContain(
      join(context.updater.currentLink, "dist", "cli.js"),
    );

    await runInstallCommand(["--root", packageRoot, "--harness", "claude-code", "--remove"], context);

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
    await runInstallCommand(["--root", packageRoot, "--harness", "claude-code"], context);
    const files = { "rerail.sqlite": "rules and history", "rerail.sqlite-wal": "wal" };
    for (const [name, text] of Object.entries(files)) await writeFile(join(context.home, name), text);

    await runInstallCommand(["--root", packageRoot, "--harness", "claude-code", "--remove"], context);
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
    await runInstallCommand(["--root", packageRoot, "--harness", "claude-code"], context);
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
    await runInstallCommand(["--root", packageRoot, "--harness", "claude-code"], context);
    await writeFile(join(context.home, "rerail.sqlite"), "live database");
    // The undo names that checkout, not the managed install `current` points at.
    const checkoutRoot = join(hostConfigHome, "..", "checkout");
    await mkdir(join(checkoutRoot, "dist"), { recursive: true });

    expect(await runInstallCommand(["--root", checkoutRoot, "--remove"], context)).toBe(0);

    expect(await readFile(join(context.home, "rerail.sqlite"), "utf8")).toBe("live database");
    expect((await readdir(hostConfigHome)).filter(name => name.startsWith(".rerail-clean-"))).toEqual([]);
  });
});

describe("rerail setup --dry-run", () => {
  it("plans the coding-agent registration from the running package before anything is installed", async () => {
    const { context, hostConfigHome, packageRoot } = await fixture();
    await mkdir(join(hostConfigHome, ".codex"), { recursive: true });
    await mkdir(join(packageRoot, "skills", "rerail"), { recursive: true });
    await writeFile(join(packageRoot, "skills", "rerail", "SKILL.md"), "Run RERAIL_COMMAND.\n");

    expect(await runSetupCommand(["--dry-run"], context)).toBe(0);

    expect(existsSync(context.home)).toBe(false);
    expect(existsSync(join(hostConfigHome, ".codex", "skills"))).toBe(false);
    expect(existsSync(join(hostConfigHome, ".claude", "settings.json"))).toBe(false);
  });
});
