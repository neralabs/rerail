import { afterEach, describe, expect, it, vi } from "vitest";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, readlink, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { runInstallCommand, type SetupContext } from "../src/setup-cli.js";
import { ManagedUpdater } from "../src/updater.js";

const temporaryDirectories: string[] = [];
const previousHome = process.env.HOME;

afterEach(async () => {
  vi.restoreAllMocks();
  if (previousHome === undefined) delete process.env.HOME;
  else process.env.HOME = previousHome;
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

interface Fixture {
  context: SetupContext;
  /** The harness config home `install` writes into; `process.env.HOME` points here for the test. */
  harnessHome: string;
  packageRoot: string;
}

/** A checkout-shaped package root plus an empty Rerail home, with Claude Code the only detected harness. */
async function fixture(): Promise<Fixture> {
  const base = await mkdtemp(join(tmpdir(), "rerail-setup-cli-"));
  temporaryDirectories.push(base);
  const packageRoot = join(base, "checkout", "rerail");
  await mkdir(join(packageRoot, "dist"), { recursive: true });
  await writeFile(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "1.0.0" }));
  const harnessHome = join(base, "home");
  await mkdir(join(harnessHome, ".claude"), { recursive: true });
  process.env.HOME = harnessHome;
  vi.spyOn(process, "cwd").mockReturnValue(base);
  const home = join(base, "rerail-home");
  return {
    context: { home, packageRoot, updater: new ManagedUpdater({ home, packageRoot, packageVersion: "1.0.0" }) },
    harnessHome,
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
    const { context, harnessHome, packageRoot } = await fixture();
    const settingsPath = join(harnessHome, ".claude", "settings.json");
    const hookPath = join(context.updater.currentLink, "dist", "claude-hook.js");

    await runInstallCommand(["--root", packageRoot, "--harness", "claude-code"], context);

    const settings = JSON.parse(await readFile(settingsPath, "utf8")) as {
      hooks: Record<string, Array<{ hooks: Array<{ command: string }> }>>;
    };
    // One stable path, not the checkout's: retiring the workspace cannot orphan the hooks.
    for (const groups of Object.values(settings.hooks)) {
      for (const group of groups) for (const hook of group.hooks) expect(hook.command).toContain(hookPath);
    }
    expect(await readFile(join(harnessHome, ".claude", "commands", "rerail.md"), "utf8")).toContain(
      join(context.updater.currentLink, "dist", "cli.js"),
    );

    await runInstallCommand(["--root", packageRoot, "--harness", "claude-code", "--remove"], context);

    expect(JSON.parse(await readFile(settingsPath, "utf8"))).toEqual({});
    expect(existsSync(join(harnessHome, ".claude", "commands", "rerail.md"))).toBe(false);
  });

  it("writes nothing under the Rerail home for a dry run", async () => {
    const { context, packageRoot } = await fixture();
    const { updater } = context;

    expect(await runInstallCommand(["--root", packageRoot, "--dry-run"], context)).toBe(0);

    expect(existsSync(updater.currentLink)).toBe(false);
    expect(existsSync(updater.launcherPath)).toBe(false);
    expect(existsSync(context.home)).toBe(false);
  });
});
