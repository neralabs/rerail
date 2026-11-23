import { readFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { commitDurably, openRerailDatabase, openSqliteDatabase } from "../../../src/engine/storage/database.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

// The store's writers commit without waiting for the disk: SQLite's `PRAGMA synchronous` of the open connection is 1
// (NORMAL), so a power cut can take the last seconds of records, which the reference tells a person; 2 (FULL) would make
// every commit wait for the disk.
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
  it("opens every writer with the write-ahead log and NORMAL, so no commit waits for the disk", async () => {
    const path = await databasePath();
    for (const open of [() => openRerailDatabase(path), () => openSqliteDatabase(path)]) {
      const database = open();
      try {
        expect(database.$client.pragma("journal_mode", { simple: true })).toBe("wal");
        expect(database.$client.pragma("synchronous", { simple: true })).toBe(NORMAL);
      } finally {
        database.$client.close();
      }
    }
  });

  it("is what the reference tells a person: a power cut can take the last seconds of records", () => {
    expect(readFileSync(REFERENCE, "utf8"), "docs/reference.md").toContain(MAY_LOSE_THE_LAST_RECORDS);
  });

  // The rare commit a power cut must not take (the remote outbox's record of a batch it sends) waits for the disk alone:
  // FULL while it commits, and the connection's own setting after, however the write ends.
  it("waits for the disk at a durable commit only, then gives the connection its own setting back", async () => {
    const database = openRerailDatabase(await databasePath());
    const client = database.$client;
    try {
      expect(commitDurably(client, () => client.pragma("synchronous", { simple: true }))).toBe(FULL);
      expect(client.pragma("synchronous", { simple: true })).toBe(NORMAL);
      expect(() => commitDurably(client, () => { throw new Error("refused"); })).toThrow("refused");
      expect(client.pragma("synchronous", { simple: true })).toBe(NORMAL);
    } finally {
      client.close();
    }
  });

  // Inside a transaction the write would commit with it, without waiting for the disk.
  it("refuses a durable commit inside an open transaction", async () => {
    const database = openRerailDatabase(await databasePath());
    const client = database.$client;
    try {
      expect(() => client.transaction(() => commitDurably(client, () => 1))()).toThrow("commitDurably must be called outside a transaction");
      expect(client.pragma("synchronous", { simple: true })).toBe(NORMAL);
    } finally {
      client.close();
    }
  });
});
