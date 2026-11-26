import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { RERAIL_MIGRATIONS, addColumn, type Migration } from "../../../src/engine/storage/migrations.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const folders: string[] = [];
const closers: Array<() => void> = [];

afterEach(async () => {
  vi.restoreAllMocks();
  for (const close of closers.splice(0).reverse()) close();
  await Promise.all(folders.splice(0).map(folder => removeFolder(folder)));
});

/** A file in this build's shape, made by a writable open. */
async function currentFile(): Promise<string> {
  const folder = await mkdtemp(join(tmpdir(), "rerail-open-beside-writer-"));
  folders.push(folder);
  const path = join(folder, "rerail.sqlite");
  openRerailDatabase(path).$client.close();
  return path;
}

/** Another connection holding the write lock (`BEGIN IMMEDIATE`), as `rerail scrub`'s VACUUM or `rerail reset` does. */
function holdWriteLock(path: string): Database.Database {
  const holder = new Database(path);
  closers.push(() => holder.close());
  holder.exec("BEGIN IMMEDIATE");
  return holder;
}

/** A step this build's file has not run, adding `rule_settings.note`, counting its runs. */
function noteStep(runs: { count: number }): Migration {
  return {
    id: "add-rule-settings-note",
    up(database) {
      runs.count += 1;
      addColumn(database, "rule_settings", "note", "text");
    },
  };
}

function opened(path: string, migrations?: readonly Migration[]): Database.Database {
  const client = openRerailDatabase(path, migrations === undefined ? {} : { migrations }).$client;
  closers.push(() => client.close());
  return client;
}

function hasNote(path: string): boolean {
  const reader = new Database(path, { readonly: true });
  try {
    return (reader.prepare("SELECT name FROM pragma_table_info('rule_settings')").pluck().all() as string[]).includes("note");
  } finally {
    reader.close();
  }
}

describe("a writable open while another connection holds the database's write lock", () => {
  // Before, every writable open took the lock to run the schema's 73 statements, so this one waited 5 s and failed
  // with "database is locked".
  it("opens a file already in this build's shape without waiting, with a writer's settings", async () => {
    const path = await currentFile();
    holdWriteLock(path);

    const client = opened(path);

    expect({
      journal: client.pragma("journal_mode", { simple: true }),
      synchronous: client.pragma("synchronous", { simple: true }),
      foreignKeys: client.pragma("foreign_keys", { simple: true }),
      secureDelete: client.pragma("secure_delete", { simple: true }),
      repositories: client.prepare("SELECT count(*) FROM repositories").pluck().get(),
    }).toEqual({ journal: "wal", synchronous: 1, foreignKeys: 1, secureDelete: 1, repositories: 0 });
  });

  it("still waits for the lock to run a step the file has not run, and runs it once the lock is given back", async () => {
    const path = await currentFile();
    const holder = holdWriteLock(path);
    const runs = { count: 0 };

    expect(() => opened(path, [...RERAIL_MIGRATIONS, noteStep(runs)])).toThrow(/database is locked/u);
    expect({ runs: runs.count, note: hasNote(path) }).toEqual({ runs: 0, note: false });

    holder.exec("COMMIT");
    opened(path, [...RERAIL_MIGRATIONS, noteStep(runs)]);
    expect({ runs: runs.count, note: hasNote(path) }).toEqual({ runs: 1, note: true });
  }, 30_000);
});

describe("two openers that both find a step due", () => {
  // The first finds the step pending without the lock; the second opens, runs the step and commits before the first
  // takes the lock. The first reads the file again under the lock and runs nothing.
  it("run the step once between them", async () => {
    const path = await currentFile();
    const runs = { count: 0 };
    const migrations = [...RERAIL_MIGRATIONS, noteStep(runs)];
    const prepare = Database.prototype.prepare;
    let other = false;
    vi.spyOn(Database.prototype, "prepare").mockImplementation(function (this: Database.Database, source: string) {
      if (source === "BEGIN IMMEDIATE" && !other) {
        other = true;
        opened(path, migrations);
      }
      return prepare.call(this, source);
    });

    opened(path, migrations);

    expect({ other, runs: runs.count, note: hasNote(path) }).toEqual({ other: true, runs: 1, note: true });
  });
});
