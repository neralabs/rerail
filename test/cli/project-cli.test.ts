import { chmodSync, existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { preUpdateSnapshotPath, runResetCommand, runRulesCommand, type SnapshotUpdater, updateWithSnapshot } from "../../src/cli/project-cli.js";
import type { UpdateResult, UpdateStatus } from "../../src/setup/index.js";
import { openRerailDatabase, recordCheckout } from "../../src/engine/index.js";

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

/**
 * The bytes of the database and its WAL, read by a child process: closing a descriptor of the file in this process
 * would drop the locks this process's connections hold on it.
 */
function databaseBytes(path: string): string {
  const script = "const fs = require('node:fs'); for (const file of process.argv.slice(1)) if (fs.existsSync(file)) process.stdout.write(fs.readFileSync(file).toString('latin1'));";
  const read = spawnSync(process.execPath, ["-e", script, path, `${path}-wal`], { encoding: "latin1", maxBuffer: 256 * 1024 * 1024 });
  if (read.status !== 0) throw new Error(read.stderr);
  return read.stdout;
}

describe("rerail reset", () => {
  // The daemon holds the database open across a reset, so no closing connection checkpoints for it.
  it("leaves no byte of a deleted event in the database file or its WAL, under a connection that keeps working", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-reset-"));
    directories.push(root);
    vi.stubEnv("RERAIL_HOME", join(root, "home"));
    const checkout = join(root, "repo");
    mkdirSync(checkout);
    vi.spyOn(process, "cwd").mockReturnValue(checkout);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const path = join(root, "home", "rerail.sqlite");
    const secret = `distinctive-${randomUUID()}`;
    const live = openRerailDatabase(path);
    try {
      const { repositoryKey } = recordCheckout(live, checkout);
      const session = live.$client.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES (?, 'omp', ?, ?, 1, 2)");
      session.run("s1", checkout, repositoryKey);
      const event = live.$client.prepare("INSERT INTO events (session_id, event_id, kind, observed_at, payload_json) VALUES (?, ?, 'user-prompt', 1, ?)");
      // One copy in the database file, one still in the WAL.
      event.run("s1", "in-file", JSON.stringify({ text: secret }));
      live.$client.pragma("wal_checkpoint(TRUNCATE)");
      event.run("s1", "in-wal", JSON.stringify({ text: secret }));
      expect(databaseBytes(path)).toContain(secret);

      expect(runResetCommand([], () => "usage")).toBe(0);

      expect(databaseBytes(path)).not.toContain(secret);
      expect(live.$client.prepare("SELECT count(*) AS n FROM events").get()).toEqual({ n: 0 });
      session.run("s2", checkout, repositoryKey);
      event.run("s2", "after-reset", "{}");
      expect(live.$client.prepare("SELECT event_id AS id FROM events").all()).toEqual([{ id: "after-reset" }]);
    } finally {
      live.$client.close();
    }
  });
});
