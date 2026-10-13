import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { preUpdateSnapshotPath, restoreDatabaseForRollback, runRulesCommand, snapshotDatabaseBeforeUpdate } from "../../src/cli/project-cli.js";
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

describe("rerail update rollback", () => {
  /** A database `rerail update` copied, then the updated version migrated with a step at `floor`, if any. */
  async function updated(floor?: number): Promise<{ path: string; stopped: string[] }> {
    const root = await mkdtemp(join(tmpdir(), "rerail-rollback-"));
    directories.push(root);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const path = join(root, "rerail.sqlite");
    const database = openRerailDatabase(path);
    database.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('before', 'before', 0, 0)");
    database.$client.close();
    snapshotDatabaseBeforeUpdate(path);
    const step = { id: "after-update", ...(floor === undefined ? {} : { floor }), up: (migrated: Parameters<typeof addColumn>[0]) => addColumn(migrated, "repositories", "note", "text") };
    const migrated = openRerailDatabase(path, { migrations: [step] });
    migrated.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('after', 'after', 0, 0)");
    migrated.$client.close();
    return { path, stopped: [] };
  }

  function repositoryKeys(path: string): string[] {
    const database = new Database(path, { readonly: true });
    try {
      return database.prepare("SELECT repository_key FROM repositories ORDER BY repository_key").pluck().all() as string[];
    } finally {
      database.close();
    }
  }

  it("puts the pre-update copy back when the update raised the schema floor, keeping the replaced database", async () => {
    const { path, stopped } = await updated(RERAIL_BASELINE_SCHEMA_VERSION + 1);
    await restoreDatabaseForRollback(path, async () => { stopped.push("daemon"); });
    expect(stopped).toEqual(["daemon"]);
    // The version rolled back to opens it again.
    openRerailDatabase(path).$client.close();
    expect(repositoryKeys(path)).toEqual(["before"]);
    expect(repositoryKeys(`${path}.before-rollback`)).toEqual(["after", "before"]);
  });

  it("keeps the database when the update changed it only additively", async () => {
    const { path, stopped } = await updated();
    await restoreDatabaseForRollback(path, async () => { stopped.push("daemon"); });
    expect(stopped).toEqual([]);
    expect(repositoryKeys(path)).toEqual(["after", "before"]);
    expect(existsSync(`${path}.before-rollback`)).toBe(false);
  });

  it("leaves everything alone when there is no copy from before the update to go back to", async () => {
    const { path, stopped } = await updated(RERAIL_BASELINE_SCHEMA_VERSION + 1);
    writeFileSync(preUpdateSnapshotPath(path), "not a database");
    const bytes = readFileSync(path);
    await restoreDatabaseForRollback(path, async () => { stopped.push("daemon"); });
    expect(stopped).toEqual([]);
    expect(readFileSync(path)).toEqual(bytes);
  });
});
