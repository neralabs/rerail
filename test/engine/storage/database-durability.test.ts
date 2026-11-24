import { readFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { commitDurably, openRerailDatabase, openSqliteDatabase } from "../../../src/engine/storage/database.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

// The store's writers open with SQLite's `PRAGMA synchronous` at 1 (NORMAL): a commit returns once it is in the
// write-ahead log, which the daemon syncs about every 5 s, so a power cut can take about the last 5 s of history, which
// the reference tells a person, beside the rule and setting changes that wait for the disk. At 2 (FULL) every commit
// would wait for the disk.
const REFERENCE = fileURLToPath(new URL("../../../docs/reference.md", import.meta.url));
const MAY_LOSE_THE_LAST_HISTORY = "A power cut or an operating system crash can take about the last 5 s of history";
const KEEPS_CHANGES = "A change you make to rules or settings, with the `rerail` command or in the dashboard, is on the disk before Rerail says it is made.";
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
  it("opens every writer with the write-ahead log and NORMAL", async () => {
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

  it("is what the reference tells a person: a power cut can take about the last 5 s of history, never a change made", () => {
    const reference = readFileSync(REFERENCE, "utf8");
    expect(reference, "docs/reference.md").toContain(MAY_LOSE_THE_LAST_HISTORY);
    expect(reference, "docs/reference.md").toContain(KEEPS_CHANGES);
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
