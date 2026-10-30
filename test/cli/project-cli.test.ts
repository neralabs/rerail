import { chmodSync, existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { preUpdateSnapshotPath, rollbackWithSnapshot, runResetCommand, runRulesCommand, type SnapshotUpdater, updateWithSnapshot } from "../../src/cli/project-cli.js";
import type { UpdateResult, UpdateStatus } from "../../src/setup/index.js";
import { RERAIL_BASELINE_SCHEMA_VERSION, addColumn, openRerailDatabase, recordCheckout, type Migration, type RerailDatabase } from "../../src/engine/index.js";

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
  async rollback(): Promise<UpdateResult> {
    [this.current, this.previous] = [this.previous!, this.current];
    return { ...this.#status(), changed: true };
  }
}

describe("rerail update and rerail update rollback", () => {
  async function database(): Promise<string> {
    const root = await mkdtemp(join(tmpdir(), "rerail-rollback-"));
    directories.push(root);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    vi.spyOn(process.stderr, "write").mockReturnValue(true);
    const path = join(root, "rerail.sqlite");
    const created = openRerailDatabase(path);
    created.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('before', 'before', 0, 0)");
    created.$client.close();
    return path;
  }

  /** The updated version's step: it runs at `floor`, if any. */
  function afterUpdate(floor?: number): Migration {
    return { id: "after-update", ...(floor === undefined ? {} : { floor }), up: db => addColumn(db, "repositories", "note", "text") };
  }

  /** The updated version's first writer: it runs a step at `floor`, if any, and records a repository. */
  function migrate(path: string, floor?: number): void {
    const migrated = openRerailDatabase(path, { migrations: [afterUpdate(floor)] });
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

  it("puts the pre-update copy back when the update raised the schema floor, keeping the replaced database", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    const stopped: string[] = [];
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    expect(await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); })).toMatchObject({ current: { version: "1.0.0" } });
    expect(stopped).toEqual(["daemon"]);
    // The version rolled back to opens it again.
    openRerailDatabase(path).$client.close();
    expect(repositoryKeys(path)).toEqual(["before"]);
    expect(repositoryKeys(`${path}.before-rollback`)).toEqual(["after", "before"]);
  });

  // Self-review round 3: a coding agent session's own connection would keep writing to the replaced file, so the copy
  // is not put back while one is open; the person is told how to finish.
  it("leaves the database in place while a session still has it open", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const session = new Database(path);
    try {
      session.pragma("journal_mode = WAL");
      await rollbackWithSnapshot(updater, path, async () => undefined);
      session.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('during', 'during', 0, 0)");
    } finally {
      session.close();
    }
    expect(repositoryKeys(path)).toEqual(["after", "before", "during"]);
    expect(existsSync(`${path}.before-rollback`)).toBe(false);
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining("coding agent sessions still have it open. Close them, then copy"));
  });

  // Self-review round 4: a connection to the file (one opened while rollback runs, or one not in WAL mode) stays on the
  // database rollback restores: the copy is written into the file itself, never renamed over it.
  it("keeps a connection to the database on the restored copy", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const session = new Database(path);
    try {
      session.pragma("journal_mode = DELETE");
      expect(session.prepare("SELECT count(*) AS n FROM repositories").get()).toEqual({ n: 2 });
      await rollbackWithSnapshot(updater, path, async () => undefined);
      expect(session.prepare("SELECT repository_key FROM repositories ORDER BY repository_key").pluck().all()).toEqual(["before"]);
      session.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('during', 'during', 0, 0)");
    } finally {
      session.close();
    }
    expect(repositoryKeys(path)).toEqual(["before", "during"]);
    expect(repositoryKeys(`${path}.before-rollback`)).toEqual(["after", "before"]);
    openRerailDatabase(path).$client.close();
  });

  // Self-review round 5: a session of the version left (an extension already running) that opened the database after
  // the save and before the copy committed a change that ended in neither file. Every Rerail opener refuses the
  // database while `rerail.sqlite.restoring` is there.
  it("refuses a session that opens the database while the copy is put back", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const refused: string[] = [];
    let saved = false;
    await rollbackWithSnapshot(updater, path, async () => {
      // Runs once rollback awaits the copy, after the save: as a session's own work would.
      setImmediate(() => {
        saved = existsSync(`${path}.before-rollback`);
        try {
          const session = openRerailDatabase(path, { migrations: [afterUpdate(RERAIL_BASELINE_SCHEMA_VERSION + 1)] });
          session.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('during', 'during', 0, 0)");
          session.$client.close();
        } catch (error) {
          refused.push((error as Error).message);
        }
      });
    });
    // Without the refusal, 'during' was committed and is in neither file.
    expect(refused).toEqual([expect.stringContaining(`Rerail is putting back a copy of ${path} for a rollback; try again in a moment`)]);
    expect(saved).toBe(true);
    expect(existsSync(`${path}.restoring`)).toBe(false);
    expect(repositoryKeys(path)).toEqual(["before"]);
    expect(repositoryKeys(`${path}.before-rollback`)).toEqual(["after", "before"]);
  });

  it("leaves the database in place for a session that opened it before the copy is put back", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const sessions: RerailDatabase[] = [];
    try {
      await rollbackWithSnapshot(updater, path, async () => {
        sessions.push(openRerailDatabase(path, { migrations: [afterUpdate(RERAIL_BASELINE_SCHEMA_VERSION + 1)] }));
      });
      for (const session of sessions) {
        session.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('during', 'during', 0, 0)");
      }
    } finally {
      for (const session of sessions) session.$client.close();
    }
    expect(repositoryKeys(path)).toEqual(["after", "before", "during"]);
    expect(existsSync(`${path}.before-rollback`)).toBe(false);
    expect(existsSync(`${path}.restoring`)).toBe(false);
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining("coding agent sessions still have it open. Close them, then copy"));
  });

  it("leaves the database in place while another rollback puts a copy back, or one was interrupted", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    writeFileSync(`${path}.restoring`, "");
    const stopped: string[] = [];
    await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); return "stopped"; });
    // Review of #49 (N8): the daemon is left running when another rollback already restores.
    expect(stopped).toEqual([]);
    expect(existsSync(`${path}.restoring`)).toBe(true);
    expect(existsSync(`${path}.before-rollback`)).toBe(false);
    expect(repositoryKeys(path)).toEqual(["after", "before"]);
    expect(vi.mocked(process.stdout.write)).not.toHaveBeenCalledWith(expect.stringContaining("now holds the copy"));
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining(
      `${path}.restoring exists: another rollback is running, or one was interrupted. Once no rollback runs, delete ${path}.restoring, then copy`,
    ));
  });

  // Self-review round 5: better-sqlite3 resolves a backup whose first step found the file busy without copying.
  it("reports a copy that did not reach the database instead of success", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    vi.spyOn(Database.prototype, "backup").mockResolvedValue({ totalPages: 0, remainingPages: 0 });
    await rollbackWithSnapshot(updater, path, async () => undefined);
    expect(repositoryKeys(path)).toEqual(["after", "before"]);
    expect(existsSync(`${path}.restoring`)).toBe(false);
    expect(vi.mocked(process.stdout.write)).not.toHaveBeenCalledWith(expect.stringContaining("now holds the copy"));
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining(
      `could not put back ${preUpdateSnapshotPath(path)} for the rolled-back version: the copy did not reach the database, which is `
      + `still at schema v${RERAIL_BASELINE_SCHEMA_VERSION + 1} (the database as it was is saved in ${path}.before-rollback). Stop Rerail, then copy`,
    ));
  });

  // Self-review round 5: VACUUM INTO created the saved database with the process umask.
  it("keeps the replaced database readable only by its owner", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const umask = process.umask(0o022);
    try {
      await rollbackWithSnapshot(updater, path, async () => undefined);
    } finally {
      process.umask(umask);
    }
    expect(repositoryKeys(`${path}.before-rollback`)).toEqual(["after", "before"]);
    expect(statSync(`${path}.before-rollback`).mode & 0o777).toBe(0o600);
  });

  // An update that finds nothing newer, or fails, must not replace the copy of the change a rollback undoes.
  it.each(["2.0.0", "broken"])("keeps the copy of the last version change through a later update to %s", async channel => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    await updateWithSnapshot(updater, path, channel).catch(() => undefined);
    expect(updater.current).toBe("2.0.0");
    await rollbackWithSnapshot(updater, path, async () => undefined);
    openRerailDatabase(path).$client.close();
    expect(repositoryKeys(path)).toEqual(["before"]);
  });

  it("keeps the database when the update changed it only additively", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    const stopped: string[] = [];
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path);
    await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); });
    expect(stopped).toEqual([]);
    expect(repositoryKeys(path)).toEqual(["after", "before"]);
    expect(existsSync(`${path}.before-rollback`)).toBe(false);
  });

  // Self-review: a version that already opens the raised floor, whose own writer had not run when it was updated
  // from, reads the database as it is; putting the copy back would drop what changed since.
  it("keeps the database when the version rolled back to already opens its raised floor", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    const stopped: string[] = [];
    await updateWithSnapshot(updater, path, "2.0.0", RERAIL_BASELINE_SCHEMA_VERSION + 1);
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); });
    expect(stopped).toEqual([]);
    expect(repositoryKeys(path)).toEqual(["after", "before"]);
    expect(existsSync(`${path}.before-rollback`)).toBe(false);
  });

  // After 1.0.0 → 2.0.0 and a rollback to 1.0.0, rolling back again returns to 2.0.0: no copy is put back for that.
  it("never puts back a copy taken for another version change", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    await rollbackWithSnapshot(updater, path, async () => undefined);
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const keys = repositoryKeys(path);
    const stopped: string[] = [];
    await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); });
    expect(updater.current).toBe("2.0.0");
    expect(stopped).toEqual([]);
    expect(repositoryKeys(path)).toEqual(keys);
  });

  it("leaves everything alone when the copy from before the update is unreadable", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    writeFileSync(preUpdateSnapshotPath(path), "not a database");
    const bytes = readFileSync(path);
    const stopped: string[] = [];
    await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); });
    expect(stopped).toEqual([]);
    expect(readFileSync(path)).toEqual(bytes);
    // Review of #49 (F15): the person is told, and the record is not matched again.
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining(
      `rerail: could not read the schema of ${preUpdateSnapshotPath(path)} (`,
    ));
    expect(existsSync(`${preUpdateSnapshotPath(path)}.json`)).toBe(false);
  });

  // Review of #49 (B1): the record is used once. A rollback after it, and another rollback back again, find no copy to
  // put back, and an earlier rollback's saved database is never replaced.
  it("puts a copy back once, and never replaces the database an earlier rollback saved", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    await rollbackWithSnapshot(updater, path, async () => undefined);
    expect(existsSync(preUpdateSnapshotPath(path))).toBe(false);
    expect(existsSync(`${preUpdateSnapshotPath(path)}.json`)).toBe(false);
    await rollbackWithSnapshot(updater, path, async () => undefined);
    expect(updater.current).toBe("2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const working = repositoryKeys(path);
    const stopped: string[] = [];
    await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); });
    expect(updater.current).toBe("1.0.0");
    expect(stopped).toEqual([]);
    expect(repositoryKeys(path)).toEqual(working);
    expect(repositoryKeys(`${path}.before-rollback`)).toEqual(["after", "before"]);

    // A later update's restore keeps its own saved database beside the first.
    const again = await database();
    const second = new FakeUpdater();
    await updateWithSnapshot(second, again, "2.0.0");
    migrate(again, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    await rollbackWithSnapshot(second, again, async () => undefined);
    await updateWithSnapshot(second, again, "3.0.0");
    const latest = new Database(again);
    latest.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('latest', 'latest', 0, 0)");
    latest.close();
    migrate(again, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    await rollbackWithSnapshot(second, again, async () => undefined);
    expect(repositoryKeys(`${again}.before-rollback`)).toEqual(["after", "before"]);
    expect(repositoryKeys(`${again}.before-rollback.2`)).toEqual(["after", "before", "latest"]);
    expect(vi.mocked(process.stdout.write)).toHaveBeenCalledWith(expect.stringContaining(`What changed since the update is in ${again}.before-rollback.2.`));
  });

  // Review of #49 (F2, F3): a version change without a kept copy leaves no record of an earlier change to match, and
  // failing to keep the copy does not fail the update.
  it.each([
    ["the copy fails", (path: string) => mkdirSync(`${preUpdateSnapshotPath(path)}.${process.pid}.tmp`)],
    ["recording it fails", (path: string) => mkdirSync(`${preUpdateSnapshotPath(path)}.json.tmp`)],
  ])("keeps no copy of an earlier change when %s on a version change", async (_, fail) => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path);
    await rollbackWithSnapshot(updater, path, async () => undefined);
    expect(existsSync(`${preUpdateSnapshotPath(path)}.json`)).toBe(true);
    fail(path);
    await expect(updateWithSnapshot(updater, path, "2.0.0")).resolves.toMatchObject({ changed: true, current: { version: "2.0.0" } });
    expect(existsSync(preUpdateSnapshotPath(path))).toBe(false);
    expect(existsSync(`${preUpdateSnapshotPath(path)}.json`)).toBe(false);
    openRerailDatabase(path, { migrations: [afterUpdate(), { id: "raise", floor: RERAIL_BASELINE_SCHEMA_VERSION + 1, up: () => {} }] }).$client.close();
    const keys = repositoryKeys(path);
    const stopped: string[] = [];
    await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); });
    expect(stopped).toEqual([]);
    expect(repositoryKeys(path)).toEqual(keys);
  });

  // Self-review round 15: when neither a new copy nor the old one's removal works (a folder this user cannot write),
  // the update has still happened: it is reported, with what to remove by hand, not failed.
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
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining(`rerail: remove ${target} and ${target}.json yourself before rolling back:`));
  });

  // Review of #49 (F4): only another connection holding the file is "sessions still have it open".
  it("reports a journal switch that fails for another reason as that reason", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const pragma = Database.prototype.pragma;
    vi.spyOn(Database.prototype, "pragma").mockImplementation(function (this: Database.Database, source: string, options?: Database.PragmaOptions) {
      if (source === "journal_mode = DELETE") throw new Database.SqliteError("disk I/O error", "SQLITE_IOERR");
      return pragma.call(this, source, options);
    });
    await rollbackWithSnapshot(updater, path, async () => undefined);
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining(
      `could not put back ${preUpdateSnapshotPath(path)} for the rolled-back version: disk I/O error.`,
    ));
    expect(vi.mocked(process.stderr.write)).not.toHaveBeenCalledWith(expect.stringContaining("still have it open"));
  });

  // Review of #49 (F7): an opener's brief read can make the first backup step copy nothing; it is tried again.
  it("tries the copy again when a backup step found the file busy", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const backup = vi.spyOn(Database.prototype, "backup").mockResolvedValueOnce({ totalPages: 0, remainingPages: 0 });
    await rollbackWithSnapshot(updater, path, async () => undefined);
    expect(backup).toHaveBeenCalledTimes(2);
    expect(repositoryKeys(path)).toEqual(["before"]);
    expect(vi.mocked(process.stdout.write)).toHaveBeenCalledWith(expect.stringContaining("now holds the copy"));
  });

  // Review of #49 (N3): the copy holds every repository's history, like the saved database.
  it("keeps the pre-update copy readable only by its owner", async () => {
    const path = await database();
    const umask = process.umask(0o022);
    try {
      await updateWithSnapshot(new FakeUpdater(), path, "2.0.0");
    } finally {
      process.umask(umask);
    }
    expect(statSync(preUpdateSnapshotPath(path)).mode & 0o777).toBe(0o600);
  });

  // Review of #49 (F8): the versions recorded are the updater's own, read under its lock; another update between a
  // separate read and this one would have recorded a change that never happened.
  it("records the version change the updater made, even when another update ran first", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    const update = updater.update.bind(updater);
    updater.update = async version => {
      await update("1.5.0");
      return update(version);
    };
    await updateWithSnapshot(updater, path, "2.0.0");
    expect(JSON.parse(readFileSync(`${preUpdateSnapshotPath(path)}.json`, "utf8"))).toMatchObject({ from: "1.5.0", to: "2.0.0" });
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    await rollbackWithSnapshot(updater, path, async () => undefined);
    expect(updater.current).toBe("1.5.0");
    expect(repositoryKeys(path)).toEqual(["before"]);
  });

  // Review of #49 (F10): a saved database VACUUM INTO could not fill is not left behind under the name later ones need.
  it("leaves no empty saved database when saving the replaced one fails", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const prepare = Database.prototype.prepare;
    vi.spyOn(Database.prototype, "prepare").mockImplementation(function (this: Database.Database, source: string) {
      if (source === "VACUUM INTO ?") throw new Database.SqliteError("database or disk is full", "SQLITE_FULL");
      return prepare.call(this, source);
    });
    await rollbackWithSnapshot(updater, path, async () => undefined);
    expect(existsSync(`${path}.before-rollback`)).toBe(false);
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(expect.stringContaining("database or disk is full. Stop Rerail, then copy"));
  });

  // Review of #49 (F9): 0.1.0-next.1's `rerail update` kept a copy (schema v1) with no record; rolling back to it
  // after this release adopted the database leaves a file next.1 refuses, so the rollback says how to put the copy back.
  it("tells how to put back a copy an older update kept without a record, when its floor is below the database's", async () => {
    const path = await database();
    const copy = new Database(preUpdateSnapshotPath(path));
    copy.pragma("user_version = 1");
    copy.exec("CREATE TABLE sessions (session_id text)");
    copy.close();
    const updater = new FakeUpdater();
    updater.previous = "0.1.0-next.1";
    const stopped: string[] = [];
    await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); });
    expect(updater.current).toBe("0.1.0-next.1");
    expect(stopped).toEqual([]);
    expect(repositoryKeys(path)).toEqual(["before"]);
    expect(vi.mocked(process.stderr.write)).toHaveBeenCalledWith(
      `rerail: ${path} is at schema v${RERAIL_BASELINE_SCHEMA_VERSION}, and ${preUpdateSnapshotPath(path)} (schema v1) was taken by an update that `
      + "recorded no version change, so Rerail does not put it back itself. If 0.1.0-next.1 refuses the database, stop Rerail, "
      + `keep ${path} elsewhere if you need what changed since that update, then copy ${preUpdateSnapshotPath(path)} to ${path}.\n`,
    );
  });

  // Review of #49 (F13): a rollback that told the person to put the copy back by hand does not put it back on a later
  // rollback of the same change, after they moved forward again.
  it("never puts back a copy a refused rollback left to the person, on a later rollback of the same change", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const session = new Database(path);
    try {
      session.pragma("journal_mode = WAL");
      await rollbackWithSnapshot(updater, path, async () => undefined);
    } finally {
      session.close();
    }
    expect(existsSync(preUpdateSnapshotPath(path))).toBe(true);
    expect(existsSync(`${preUpdateSnapshotPath(path)}.json`)).toBe(false);
    await rollbackWithSnapshot(updater, path, async () => undefined);
    expect(updater.current).toBe("2.0.0");
    const latest = new Database(path);
    latest.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('latest', 'latest', 0, 0)");
    latest.close();
    const stopped: string[] = [];
    await rollbackWithSnapshot(updater, path, async () => { stopped.push("daemon"); });
    expect(stopped).toEqual([]);
    expect(repositoryKeys(path)).toEqual(["after", "before", "latest"]);
  });

  // Review of #49 (F14): a rollback killed while restoring left its file behind; it names a process that is gone, so
  // it holds nothing back, and the next rollback takes it over. (N8) The message says the daemon was stopped.
  it("takes over a restoring file a killed rollback left, and says it stopped the daemon", async () => {
    const path = await database();
    const updater = new FakeUpdater();
    await updateWithSnapshot(updater, path, "2.0.0");
    migrate(path, RERAIL_BASELINE_SCHEMA_VERSION + 1);
    const gone = spawnSync(process.execPath, ["-e", ""]).pid;
    writeFileSync(`${path}.restoring`, `${gone}\n`);
    openRerailDatabase(path, { readonly: true, migrations: [afterUpdate(RERAIL_BASELINE_SCHEMA_VERSION + 1)] }).$client.close();
    await rollbackWithSnapshot(updater, path, async () => "stopped");
    expect(repositoryKeys(path)).toEqual(["before"]);
    expect(existsSync(`${path}.restoring`)).toBe(false);
    expect(vi.mocked(process.stdout.write)).toHaveBeenCalledWith(expect.stringContaining(
      "now holds the copy taken before the update",
    ));
    expect(vi.mocked(process.stdout.write)).toHaveBeenCalledWith(expect.stringContaining(
      "Rerail's daemon was stopped for this; rerail daemon start starts it again.",
    ));
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
