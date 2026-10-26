import { afterEach, describe, expect, it } from "vitest";
import { existsSync } from "node:fs";
import { lstat, mkdir, mkdtemp, readFile, readlink, readdir, rm, stat, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import {
  ManagedUpdater,
  compareSemver,
  launcherShim,
  validateManagedInstallation,
  type Installation,
  type PackageRegistry,
} from "../../src/setup/updater.js";
import { ENTRY_POINTS } from "../../src/engine/index.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

class FakeRegistry implements PackageRegistry {
  readonly resolvedChannels: string[] = [];
  readonly installedVersions: string[] = [];

  constructor(public availableVersion: string) {}

  async resolveVersion(channel: string): Promise<string> {
    this.resolvedChannels.push(channel);
    return this.availableVersion;
  }

  async installVersion(version: string, prefix: string): Promise<void> {
    this.installedVersions.push(version);
    const packageRoot = join(prefix, "node_modules", "@neralabs", "rerail");
    await mkdir(packageRoot, { recursive: true });
    await writeFile(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version }));
  }
}

async function fixture(): Promise<{ home: string; packageRoot: string }> {
  const root = await mkdtemp(join(tmpdir(), "rerail-updater-"));
  temporaryDirectories.push(root);
  const packageRoot = join(root, "bundled");
  await mkdir(packageRoot);
  await writeFile(join(packageRoot, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "1.0.0" }));
  return { home: join(root, "home"), packageRoot };
}

async function assertPackageIdentity(installation: Installation): Promise<void> {
  const manifest = JSON.parse(await readFile(join(installation.packageRoot, "package.json"), "utf8")) as Record<
    string,
    unknown
  >;
  expect(manifest).toMatchObject({ name: "@neralabs/rerail", version: installation.version });
}

describe("semantic version ordering", () => {
  it("orders releases and prereleases without accepting loose versions", () => {
    expect(compareSemver("1.2.0", "1.1.9")).toBe(1);
    expect(compareSemver("1.2.0-beta.2", "1.2.0-beta.11")).toBe(-1);
    expect(compareSemver("1.2.0", "1.2.0-beta.11")).toBe(1);
    expect(() => compareSemver("v1.2.0", "1.2.0")).toThrow("invalid semantic version");
  });
});

describe("managed launcher shims", () => {
  it("writes a Windows batch launcher with argument forwarding and percent-safe paths", () => {
    const shim = launcherShim(
      String.raw`C:\Users\100% User\.rerail\current`,
      String.raw`C:\Program Files\nodejs\node.exe`,
      "win32",
    );
    expect(shim).toContain('set "RERAIL_NODE=C:\\Program Files\\nodejs\\node.exe"');
    expect(shim).toContain(String.raw`"C:\Users\100%% User\.rerail\current\dist\cli\cli.js" %*`);
    expect(shim.endsWith("\r\n")).toBe(true);
  });
});

describe("ManagedUpdater", () => {
  it("installs, validates, atomically activates, and rolls back a newer version", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.1.0");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      now: () => new Date("2026-09-01T12:00:00.000Z"),
      validateInstallation: assertPackageIdentity,
    });

    const updated = await updater.update("beta");
    expect(updated).toMatchObject({
      changed: true,
      channel: "beta",
      current: { version: "1.1.0" },
      previous: { version: "1.0.0", packageRoot },
      lastCheckAt: "2026-09-01T12:00:00.000Z",
    });
    expect(registry.resolvedChannels).toEqual(["beta"]);
    expect(registry.installedVersions).toEqual(["1.1.0"]);
    await expect(readFile(join(updated.current.packageRoot, "package.json"), "utf8")).resolves.toContain('"1.1.0"');

    const rolledBack = await updater.rollback();
    expect(rolledBack).toMatchObject({
      changed: true,
      current: { version: "1.0.0", packageRoot },
      previous: { version: "1.1.0" },
    });
  });

  it("does not reinstall or downgrade when the selected channel is not newer", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.0.0");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      now: () => new Date("2026-09-01T12:00:00.000Z"),
      validateInstallation: assertPackageIdentity,
    });

    const result = await updater.update();
    expect(result.changed).toBe(false);
    expect(result.current).toEqual({ version: "1.0.0", packageRoot });
    expect(registry.installedVersions).toEqual([]);
  });

  it("keeps the active version unchanged when staged validation fails", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.1.0");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      now: () => new Date("2026-09-01T12:00:00.000Z"),
      validateInstallation: async () => {
        throw new Error("health check failed");
      },
    });

    await expect(updater.update()).rejects.toThrow("health check failed");
    expect(await updater.status()).toMatchObject({
      current: { version: "1.0.0", packageRoot },
      lastCheckAt: "2026-09-01T12:00:00.000Z",
    });
  });

  it("keeps the active link and launcher intact when an update fails after staging", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.0.0");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      validateInstallation: async (installation) => {
        await assertPackageIdentity(installation);
        if (installation.version === "1.1.0") throw new Error("staged runtime failed");
      },
    });
    const installed = await updater.install();
    const originalLauncher = await readFile(updater.launcherPath, "utf8");

    registry.availableVersion = "1.1.0";
    await expect(updater.update()).rejects.toThrow("staged runtime failed");
    const status = await updater.status();
    expect(status.current).toEqual(installed.current);
    expect(status.previous).toBeUndefined();
    expect(await readlink(updater.currentLink)).toBe(installed.current.packageRoot);
    expect(await readFile(updater.launcherPath, "utf8")).toBe(originalLauncher);
    expect(await readdir(updater.versionsRoot)).toEqual(["1.0.0"]);
  });

  it("repairs the current installation when no newer version exists", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.0.0");
    const validated: string[] = [];
    const repaired: string[] = [];
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      validateInstallation: async (installation) => {
        await assertPackageIdentity(installation);
        validated.push(installation.packageRoot);
        if (repaired.length === 0 && validated.length > 1) throw new Error("NODE_MODULE_VERSION 137");
      },
      repairInstallation: async (root) => {
        repaired.push(root);
        return { prefix: dirname(dirname(dirname(root))), rebuilt: true };
      },
    });
    await updater.install();
    const current = (await updater.status()).current;
    const installsBeforeUpdate = registry.installedVersions.length;

    const result = await updater.update();
    expect(result.changed).toBe(false);
    expect(repaired).toEqual([current.packageRoot]);
    expect(validated.filter(root => root === current.packageRoot)).toHaveLength(2);
    expect(registry.installedVersions).toHaveLength(installsBeforeUpdate);
  });

  it("surfaces an unrepairable current installation instead of reporting no update", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.0.0");
    let installed = false;
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      validateInstallation: async () => {
        if (installed) throw new Error("NODE_MODULE_VERSION 137");
      },
      repairInstallation: async () => {
        throw new Error("better-sqlite3 in /versions/1.0.0 cannot load under Node 25.2.1 (ABI 141) and npm rebuild did not fix it.");
      },
    });
    await updater.install();
    installed = true;

    await expect(updater.update()).rejects.toThrow("npm rebuild did not fix it");
    expect((await updater.status()).lastCheckAt).toBeDefined();
  });

  it("rebuilds the SQLite binding of an installed version that fails validation, then validates again", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.1.0");
    const validated: string[] = [];
    const repaired: string[] = [];
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      validateInstallation: async (installation) => {
        await assertPackageIdentity(installation);
        validated.push(installation.version);
        if (installation.version === "1.1.0" && repaired.length === 0) throw new Error("NODE_MODULE_VERSION 137");
      },
      repairInstallation: async (root) => {
        repaired.push(root);
        return { prefix: join(home, "versions", "1.1.0"), rebuilt: true };
      },
    });
    await updater.install();
    await registry.installVersion("1.1.0", join(home, "versions", "1.1.0"));
    const installsBeforeUpdate = registry.installedVersions.length;

    const status = await updater.update();
    expect(status.current.version).toBe("1.1.0");
    expect(repaired).toEqual([join(home, "versions", "1.1.0", "node_modules", "@neralabs", "rerail")]);
    expect(validated.filter(version => version === "1.1.0")).toEqual(["1.1.0", "1.1.0"]);
    expect(registry.installedVersions).toHaveLength(installsBeforeUpdate);
  });

  it("surfaces the repair's own message when the binding cannot be rebuilt", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.1.0");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      validateInstallation: async (installation) => {
        if (installation.version === "1.1.0") throw new Error("NODE_MODULE_VERSION 137");
      },
      repairInstallation: async () => {
        throw new Error("better-sqlite3 in /versions/1.1.0 cannot load under Node 25.2.1 (ABI 141) and npm rebuild did not fix it.");
      },
    });
    await updater.install();
    await registry.installVersion("1.1.0", join(home, "versions", "1.1.0"));

    await expect(updater.update()).rejects.toThrow("npm rebuild did not fix it");
    expect((await updater.status()).current.version).toBe("1.0.0");
  });

  it("keeps the original validation failure when the binding was not the problem", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.1.0");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      validateInstallation: async (installation) => {
        if (installation.version === "1.1.0") throw new Error("health check failed");
      },
      repairInstallation: async (root) => ({ prefix: dirname(dirname(dirname(root))), rebuilt: false }),
    });
    await updater.install();
    await registry.installVersion("1.1.0", join(home, "versions", "1.1.0"));

    await expect(updater.update()).rejects.toThrow("health check failed");
  });

  it("refuses a live lease regardless of the lock directory age", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.1.0");
    await mkdir(join(home, "update.lock"), { recursive: true });
    await writeFile(
      join(home, "update.lock", "lease.json"),
      JSON.stringify({
        owner: "another-updater",
        expiresAt: "2026-09-01T12:15:00.000Z",
      }),
    );
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      now: () => new Date("2026-09-01T12:00:00.000Z"),
      validateInstallation: assertPackageIdentity,
    });

    await expect(updater.update()).rejects.toThrow("Another Rerail update is already running");
    expect(registry.installedVersions).toEqual([]);
    await expect(readFile(join(home, "update.lock", "lease.json"), "utf8")).resolves.toContain("another-updater");
  });

  it("does not release a lease replaced by another owner", async () => {
    const { home, packageRoot } = await fixture();
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry: new FakeRegistry("1.1.0"),
      now: () => new Date("2026-09-01T12:00:00.000Z"),
      validateInstallation: async (installation) => {
        await assertPackageIdentity(installation);
        await writeFile(
          join(home, "update.lock", "lease.json"),
          JSON.stringify({
            owner: "different-updater",
            expiresAt: "2026-09-01T12:15:00.000Z",
          }),
        );
      },
    });

    await updater.update();
    await expect(readFile(join(home, "update.lock", "lease.json"), "utf8")).resolves.toContain("different-updater");
  });
});

/**
 * An installed release from before set-up, as 0.1.0-next.0 behaves under the health check: `state` answers "never",
 * `start` rejects `--answer` as an unknown option, and only a started folder opens a session.
 */
async function preSetupRelease(): Promise<Installation> {
  const root = await mkdtemp(join(tmpdir(), "rerail-pre-setup-release-"));
  temporaryDirectories.push(root);
  await Promise.all([ENTRY_POINTS.index, ENTRY_POINTS.cli, ENTRY_POINTS.eventStream].map(path => mkdir(dirname(join(root, path)), { recursive: true })));
  await writeFile(join(root, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "0.1.0-next.0", type: "module" }));
  await writeFile(join(root, ENTRY_POINTS.index), "export function nativeHealth() { return { ok: true }; }\n");
  await writeFile(join(root, ENTRY_POINTS.cli), [
    'import { mkdirSync } from "node:fs";',
    'import { join } from "node:path";',
    "const [command, ...args] = process.argv.slice(2);",
    'if (command === "state") process.stdout.write("never\\n");',
    'else if (command === "start" && args.includes("--answer")) {',
    '  process.stderr.write("rerail: Unknown start option: --answer\\n");',
    "  process.exitCode = 1;",
    '} else if (command === "start") mkdirSync(join(process.env.RERAIL_HOME, "started"), { recursive: true });',
    "else process.exitCode = 1;",
    "",
  ].join("\n"));
  await writeFile(join(root, ENTRY_POINTS.eventStream), [
    'import { existsSync, mkdirSync, readFileSync } from "node:fs";',
    'import { join } from "node:path";',
    'const request = JSON.parse(readFileSync(0, "utf8").trimEnd().split("\\n").at(-1));',
    'if (existsSync(join(process.env.RERAIL_HOME, "started"))) mkdirSync(join(process.env.RERAIL_STORAGE_ROOT, "session"), { recursive: true });',
    'process.stdout.write(JSON.stringify({ eventId: request.event.eventId, deliveries: [] }) + "\\n");',
    "",
  ].join("\n"));
  return { version: "0.1.0-next.0", packageRoot: root };
}

describe("rollback to a release from before set-up", () => {
  it("passes the health check by starting the release without --answer", async () => {
    const { home, packageRoot } = await fixture();
    const previous = await preSetupRelease();
    await mkdir(home, { recursive: true });
    await writeFile(join(home, "update-state.json"), JSON.stringify({
      schemaVersion: 1,
      channel: "latest",
      current: { version: "1.0.0", packageRoot },
      previous,
    }));
    // No validateInstallation: the real health check runs against the release.
    const updater = new ManagedUpdater({ home, packageRoot, packageVersion: "1.0.0", registry: new FakeRegistry("1.0.0") });

    expect(await updater.rollback()).toMatchObject({ changed: true, current: previous, previous: { version: "1.0.0" } });
  }, 30_000);
});

describe("ManagedUpdater.install (setup bootstrap)", () => {
  it("materializes the running version under versions/, links current, and writes the launcher", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.0.0");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      validateInstallation: assertPackageIdentity,
    });

    const installed = await updater.install();
    expect(installed.changed).toBe(true);
    expect(installed.current).toEqual({
      version: "1.0.0",
      packageRoot: join(home, "versions", "1.0.0", "node_modules", "@neralabs", "rerail"),
    });
    // The npx cache / checkout it ran from is not a rollback target.
    expect(installed.previous).toBeUndefined();
    expect(registry.installedVersions).toEqual(["1.0.0"]);

    expect(await readlink(updater.currentLink)).toBe(installed.current.packageRoot);
    const shim = await readFile(updater.launcherPath, "utf8");
    expect(shim.startsWith("#!/bin/sh\n")).toBe(true);
    expect(shim).toContain(`'${join(updater.currentLink, "dist", "cli", "cli.js")}'`);
    expect(shim).toContain(`NODE='${process.execPath}'`);
    expect((await stat(updater.launcherPath)).mode & 0o111).toBe(0o111);
    expect(shim).toBe(launcherShim(updater.currentLink, process.execPath));

    // Re-running is a no-op: same root, nothing reinstalled, launcher untouched.
    const again = await updater.install();
    expect(again.changed).toBe(false);
    expect(registry.installedVersions).toEqual(["1.0.0"]);
    expect(await readlink(updater.currentLink)).toBe(installed.current.packageRoot);
  });

  it("replaces a dangling Windows current junction", async () => {
    const { home, packageRoot } = await fixture();
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      platform: "win32",
      registry: new FakeRegistry("1.0.0"),
      validateInstallation: assertPackageIdentity,
    });
    await mkdir(home, { recursive: true });
    await symlink(join(home, "missing-version"), updater.currentLink, "junction");

    const installed = await updater.install();

    expect(await readlink(updater.currentLink)).toBe(installed.current.packageRoot);
    expect((await lstat(updater.currentLink)).isSymbolicLink()).toBe(true);
  });

  it("keeps a managed previous version for rollback and re-points current on update", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.0.0");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      validateInstallation: assertPackageIdentity,
    });
    const first = await updater.install();

    registry.availableVersion = "1.1.0";
    const updated = await updater.update();
    expect(updated.previous).toEqual(first.current);
    expect(await readlink(updater.currentLink)).toBe(updated.current.packageRoot);

    const rolledBack = await updater.rollback();
    expect(rolledBack.current).toEqual(first.current);
    expect(await readlink(updater.currentLink)).toBe(first.current.packageRoot);

    // A later `setup` from a newer npx copy keeps the managed previous.
    const newer = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.2.0",
      registry,
      validateInstallation: assertPackageIdentity,
    });
    const bootstrapped = await newer.install();
    expect(bootstrapped.current.version).toBe("1.2.0");
    expect(bootstrapped.previous).toEqual(first.current);
  });

  it("leaves no Rerail home behind when a first install fails validation", async () => {
    const { home, packageRoot } = await fixture();
    const registry = new FakeRegistry("1.0.0");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry,
      validateInstallation: async () => {
        throw new Error("native health failed");
      },
    });
    await expect(updater.install()).rejects.toThrow("native health failed");
    // Set-up then says nothing was changed: not even an empty versions/ or Rerail home remains.
    expect(existsSync(home)).toBe(false);
  });

  it("removes only the empty versions/ it made when an install into an existing home fails", async () => {
    const { home, packageRoot } = await fixture();
    await mkdir(home);
    await writeFile(join(home, "keep.json"), "{}");
    const updater = new ManagedUpdater({
      home,
      packageRoot,
      packageVersion: "1.0.0",
      registry: new FakeRegistry("1.0.0"),
      validateInstallation: async () => {
        throw new Error("native health failed");
      },
    });
    await expect(updater.install()).rejects.toThrow("native health failed");
    expect(await readdir(home)).toEqual(["keep.json"]);
  });
});

/**
 * A staged package that passes the identity, engine and CLI checks, with `eventStream` as its
 * `dist/harness/event-stream.js`. The CLI answers `state` with `never` and accepts `start`.
 */
async function candidate(eventStream: readonly string[]): Promise<Installation> {
  const root = await mkdtemp(join(tmpdir(), "rerail-candidate-"));
  temporaryDirectories.push(root);
  const dist = join(root, "dist");
  await mkdir(join(dist, "cli"), { recursive: true });
  await mkdir(join(dist, "harness"), { recursive: true });
  await writeFile(join(root, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "2.0.0", type: "module" }));
  await writeFile(join(dist, "index.js"), "export function nativeHealth() { return { ok: true }; }\n");
  await writeFile(join(dist, "cli", "cli.js"), 'if (process.argv[2] === "state") process.stdout.write("never\\n");\n');
  await writeFile(join(dist, "harness", "event-stream.js"), [
    'import { mkdirSync, readFileSync } from "node:fs";',
    'import { join } from "node:path";',
    'const [preface, request] = readFileSync(0, "utf8").trimEnd().split("\\n");',
    ...eventStream,
    "",
  ].join("\n"));
  return { version: "2.0.0", packageRoot: root };
}

describe("validateManagedInstallation", () => {
  // A build that speaks another event protocol rejects this build's preface with an error frame and exits 0. It is
  // refused for that reply even when it opened a session, so the update or rollback never activates it.
  it("refuses a candidate whose event stream rejects this build's preface", async () => {
    const installation = await candidate([
      'mkdirSync(join(process.env.RERAIL_STORAGE_ROOT, "session"), { recursive: true });',
      'if (preface !== "EARLIER-EVENTS/1") {',
      '  process.stdout.write(JSON.stringify({ eventId: null, error: { code: "unsupported-version", message: "Expected EARLIER-EVENTS/1" } }) + "\\n");',
      "  process.exit(0);",
      "}",
    ]);
    await expect(validateManagedInstallation(installation)).rejects.toThrow("Installed event stream returned an invalid health response");
  });

  it("refuses a candidate whose event stream answers without opening a session", async () => {
    const installation = await candidate([
      'process.stdout.write(JSON.stringify({ eventId: JSON.parse(request).event.eventId, deliveries: [] }) + "\\n");',
    ]);
    await expect(validateManagedInstallation(installation)).rejects.toThrow("Installed event stream answered without opening a session");
  });

  it("accepts a candidate whose event stream opens a session and answers the health event", async () => {
    const installation = await candidate([
      'mkdirSync(join(process.env.RERAIL_STORAGE_ROOT, "session"), { recursive: true });',
      'process.stdout.write(JSON.stringify({ eventId: JSON.parse(request).event.eventId, deliveries: [] }) + "\\n");',
    ]);
    await expect(validateManagedInstallation(installation)).resolves.toBeUndefined();
  });
});
