import { mkdtemp, rm } from "node:fs/promises";
import { createRequire } from "node:module";
import type * as Sqlite from "node:sqlite";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/** Every better-sqlite3 connection opened; each loads the addon from the version folder the uninstall deletes. */
const opened = vi.hoisted(() => ({ count: 0 }));
vi.mock("better-sqlite3", () => ({
  default: vi.fn(function openBetterSqlite() {
    opened.count += 1;
    throw new Error("better-sqlite3 loaded its addon into the uninstall");
  }),
}));

import { recordedCheckouts } from "../../src/cli/setup-cli.js";

describe("the checkouts `install --remove --delete-data` reads before deleting the database", () => {
  const roots: string[] = [];
  afterEach(async () => {
    await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
  });

  it("are read without loading better-sqlite3, whose DLL Windows will not let the uninstall delete once loaded", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-recorded-"));
    roots.push(root);
    const path = join(root, "rerail.sqlite");
    const sqlite: typeof Sqlite = createRequire(import.meta.url)("node:sqlite");
    const database = new sqlite.DatabaseSync(path);
    database.exec("CREATE TABLE checkouts (checkout_path text PRIMARY KEY NOT NULL)");
    database.prepare("INSERT INTO checkouts VALUES (?), (?)").run(join(root, "b"), join(root, "a"));
    database.close();

    expect(recordedCheckouts(path)).toEqual({ checkouts: [join(root, "a"), join(root, "b")] });
    expect(opened.count).toBe(0);
    expect(recordedCheckouts(join(root, "missing.sqlite"))).toEqual({ checkouts: [] });
  });
});
