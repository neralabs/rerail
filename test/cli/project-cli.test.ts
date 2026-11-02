import { chmodSync, existsSync, mkdirSync, statSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { preUpdateSnapshotPath, runRulesCommand, type SnapshotUpdater, updateWithSnapshot } from "../../src/cli/project-cli.js";
import type { UpdateResult, UpdateStatus } from "../../src/setup/index.js";
import { defaultRerailDatabasePath, loadRuleCatalog, openRerailDatabase, upsertLibraryRule } from "../../src/engine/index.js";
import { MOCK_CODE_RULES } from "../../src/cli/index.js";

const directories: string[] = [];

afterEach(async () => {
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

describe("rerail rules export", () => {
  it("creates the folder it is told to write into", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-rules-export-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const output = join(root, "new-dir", "rules.json");

    expect(runRulesCommand(["export", "--output", output], () => "usage")).toBe(0);
    expect(existsSync(output)).toBe(true);
  });
});

describe("rerail rules import", () => {
  it("says what runs in Log after an import, and with --approve approves each imported rule as it is imported", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-rules-import-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const rule = MOCK_CODE_RULES[0]!;
    const db = openRerailDatabase(defaultRerailDatabasePath());
    upsertLibraryRule(db, loadRuleCatalog(), rule, "code");
    db.$client.close();
    let output = "";
    const io = { stdout: { write: (text: string) => (output += text) } };
    const dump = join(root, "rules.json");
    expect(runRulesCommand(["export", "--output", dump], () => "usage", io)).toBe(0);

    output = "";
    expect(runRulesCommand(["import", dump], () => "usage", io)).toBe(0);
    expect(output).toContain("1 imported rule runs in Log until a person approves it: rerail rules approve\n");

    output = "";
    expect(runRulesCommand(["import", dump, "--approve"], () => "usage", io)).toBe(0);
    expect(output).toContain(`● Approved ${rule.id}: it runs in the mode it is set to from the next edit.\n`);
    expect(output).not.toContain("runs in Log");
  });
});

/** The managed updater, by version: `update` moves to the version it is given, finds nothing newer, or fails. */
class FakeUpdater implements SnapshotUpdater {
  current = "1.0.0";
  previous: string | undefined;
  #status(): UpdateStatus {
    const previous = this.previous === undefined ? {} : { previous: { version: this.previous, packageRoot: `/v/${this.previous}` } };
    return { channel: "latest", current: { version: this.current, packageRoot: `/v/${this.current}` }, ...previous };
  }
  async update(version?: string): Promise<UpdateResult> {
    if (version === "broken") throw new Error("Installed package identity mismatch");
    if (version === undefined || version === this.current) return { ...this.#status(), changed: false };
    this.previous = this.current;
    this.current = version;
    return { ...this.#status(), changed: true };
  }
}

describe("rerail update", () => {
  async function database(): Promise<string> {
    const root = await mkdtemp(join(tmpdir(), "rerail-update-"));
    directories.push(root);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const path = join(root, "rerail.sqlite");
    const created = openRerailDatabase(path);
    created.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('before', 'before', 0, 0)");
    created.$client.close();
    return path;
  }

  /** The updated version's first writer: it records a repository. */
  function migrate(path: string): void {
    const migrated = openRerailDatabase(path);
    migrated.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('after', 'after', 0, 0)");
    migrated.$client.close();
  }

  function repositoryKeys(path: string): string[] {
    const opened = new Database(path, { readonly: true });
    try {
      return opened.prepare("SELECT repository_key FROM repositories ORDER BY repository_key").pluck().all() as string[];
    } finally {
      opened.close();
    }
  }

  it("keeps a copy of the database as it was before the version change, readable only by its owner", async () => {
    const path = await database();
    const umask = process.umask(0o022);
    try {
      await expect(updateWithSnapshot(new FakeUpdater(), path, "2.0.0")).resolves.toMatchObject({ changed: true, current: { version: "2.0.0" } });
    } finally {
      process.umask(umask);
    }
    migrate(path);
    expect(repositoryKeys(path)).toEqual(["after", "before"]);
    expect(repositoryKeys(preUpdateSnapshotPath(path))).toEqual(["before"]);
    expect(statSync(preUpdateSnapshotPath(path)).mode & 0o777).toBe(0o600);
    expect(vi.mocked(process.stdout.write)).toHaveBeenCalledWith(`Copied ${path} to ${preUpdateSnapshotPath(path)}\n`);
  });

  // An update that finds nothing newer, or fails, must not replace the copy taken before the last version change.
  it.each(["2.0.0", "broken"])("keeps the copy of the last version change through a later update to %s", async channel => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path);
    await updateWithSnapshot(updater, path, channel).catch(() => undefined);
    expect(updater.current).toBe("2.0.0");
    expect(repositoryKeys(preUpdateSnapshotPath(path))).toEqual(["before"]);
    expect(existsSync(`${preUpdateSnapshotPath(path)}.${process.pid}.tmp`)).toBe(false);
  });

  it("copies nothing when there is no database yet", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-update-"));
    directories.push(root);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const path = join(root, "rerail.sqlite");
    await expect(updateWithSnapshot(new FakeUpdater(), path, "2.0.0")).resolves.toMatchObject({ changed: true });
    expect(existsSync(preUpdateSnapshotPath(path))).toBe(false);
  });

  // A version change without a kept copy leaves no copy of an earlier change behind, and failing to keep the copy
  // does not fail the update.
  it("keeps no copy of an earlier change when the copy fails on a version change", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    expect(existsSync(preUpdateSnapshotPath(path))).toBe(true);
    mkdirSync(`${preUpdateSnapshotPath(path)}.${process.pid}.tmp`);
    await expect(updateWithSnapshot(updater, path, "3.0.0")).resolves.toMatchObject({ changed: true, current: { version: "3.0.0" } });
    expect(existsSync(preUpdateSnapshotPath(path))).toBe(false);
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining(`rerail: could not copy ${path} before the update:`));
  });

  // When neither a new copy nor the old one's removal works (a folder this user cannot write), the update has still
  // happened: it is reported, with what to remove by hand, not failed.
  it("reports an old copy it cannot remove instead of failing the update", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    chmodSync(dirname(path), 0o500);
    try {
      await expect(updateWithSnapshot(updater, path, "3.0.0")).resolves.toMatchObject({ changed: true, current: { version: "3.0.0" } });
    } finally {
      chmodSync(dirname(path), 0o700);
    }
    const target = preUpdateSnapshotPath(path);
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining(`rerail: ${target} is from an earlier update; remove it yourself:`));
  });
});
