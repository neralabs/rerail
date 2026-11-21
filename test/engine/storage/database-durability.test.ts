import { readFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase, openSqliteDatabase } from "../../../src/engine/storage/database.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

// What the reference tells a person the store keeps through a power cut is what its writers' setting gives: SQLite's
// `PRAGMA synchronous` of the open connection, 1 (NORMAL) when the last seconds of records may go, 2 (FULL) when every
// commit waits for the disk. A change of the setting without the reference's sentence, or the other way round, fails.
const REFERENCE = fileURLToPath(new URL("../../../docs/reference.md", import.meta.url));
const MAY_LOSE_THE_LAST_RECORDS = "a power cut or an operating system crash can take the last seconds of records";
const NORMAL = 1;
const FULL = 2;

const folders: string[] = [];
afterEach(async () => {
  await Promise.all(folders.splice(0).map(folder => removeFolder(folder)));
});

async function databasePath(): Promise<string> {
  const folder = await mkdtemp(join(tmpdir(), "rerail-durability-"));
  folders.push(folder);
  return join(folder, "rerail.sqlite");
}

describe("what the store keeps through a power cut", () => {
  const promised = readFileSync(REFERENCE, "utf8").includes(MAY_LOSE_THE_LAST_RECORDS) ? NORMAL : FULL;

  it("is what the reference says: every writer opens with the synchronous setting that keeps it", async () => {
    const path = await databasePath();
    for (const open of [() => openRerailDatabase(path), () => openSqliteDatabase(path)]) {
      const database = open();
      try {
        expect(database.$client.pragma("journal_mode", { simple: true })).toBe("wal");
        expect(database.$client.pragma("synchronous", { simple: true })).toBe(promised);
      } finally {
        database.$client.close();
      }
    }
  });
});
