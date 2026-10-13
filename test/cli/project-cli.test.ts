import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { preUpdateSnapshotPath, rollbackWithSnapshot, runRulesCommand, type SnapshotUpdater, updateWithSnapshot } from "../../src/cli/project-cli.js";
import type { UpdateResult, UpdateStatus } from "../../src/setup/index.js";
import { RERAIL_BASELINE_SCHEMA_VERSION, addColumn, openRerailDatabase } from "../../src/engine/index.js";

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
  async status(): Promise<UpdateStatus> {
    return this.#status();
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

  /** The updated version's first writer: it runs a step at `floor`, if any, and records a repository. */
  function migrate(path: string, floor?: number): void {
    const step = { id: "after-update", ...(floor === undefined ? {} : { floor }), up: (db: Parameters<typeof addColumn>[0]) => addColumn(db, "repositories", "note", "text") };
    const migrated = openRerailDatabase(path, { migrations: [step] });
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
  });
});
