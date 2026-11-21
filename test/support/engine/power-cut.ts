import { copyFileSync, existsSync, mkdtempSync, readFileSync, rmSync, truncateSync, writeFileSync } from "node:fs";
import { endianness, tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { removeFolderSync } from "../platform/remove-folder.js";

// The state a power cut or an operating system crash leaves the Rerail database in when its writers commit with
// `synchronous = NORMAL`: the database file as the last checkpoint wrote it, beside a write-ahead log whose tail the
// operating system never wrote back. SQLite's recovery reads the log from its start, frame by frame, and keeps every
// transaction up to the last commit frame it can read whole: what it gives back is the database as it stood at one
// earlier commit. Cutting the log file at a commit frame's end is the same state, read by the same recovery: it is the
// log the disk would hold had the operating system written back only that much of it. A checkpoint syncs the log
// before it copies frames into the database file, so a cut is never earlier than the frames a checkpoint has copied.
// The session's own files are never synced either: a test keeps them as they were, or cuts their last lines too.
//
// Call these with no connection open on the database, as after a power cut no process holds it: a writer that closes
// last checkpoints the log into the file and removes it, and a connection still open keeps its own index of the log.

const WAL_HEADER_BYTES = 32;
const FRAME_HEADER_BYTES = 24;
/** Where the WAL index (`-shm`) keeps `nBackfill`: after its two 48-byte headers. */
const BACKFILL_OFFSET = 96;

/** The log's byte length after each commit it holds, oldest first: the places a power cut can cut it. */
export function walCommits(databasePath: string): number[] {
  const log = readFileSync(`${databasePath}-wal`);
  if (log.length < WAL_HEADER_BYTES) return [];
  const pageSize = log.readUInt32BE(8);
  const salts = [log.readUInt32BE(16), log.readUInt32BE(20)];
  const commits: number[] = [];
  for (let at = WAL_HEADER_BYTES; at + FRAME_HEADER_BYTES + pageSize <= log.length; at += FRAME_HEADER_BYTES + pageSize) {
    // A frame of an earlier generation of the log, which a checkpoint restarted, carries other salts: the log ends there.
    if (log.readUInt32BE(at + 8) !== salts[0] || log.readUInt32BE(at + 12) !== salts[1]) break;
    // A commit frame records the database's size in pages; every other frame records 0.
    if (log.readUInt32BE(at + 4) !== 0) commits.push(at + FRAME_HEADER_BYTES + pageSize);
  }
  return commits;
}

/** How many frames of the log a checkpoint has copied into the database file: `nBackfill` of the WAL index. */
function backfilledFrames(databasePath: string): number {
  const indexPath = `${databasePath}-shm`;
  if (!existsSync(indexPath)) return 0;
  const index = readFileSync(indexPath);
  if (index.length < BACKFILL_OFFSET + 4) return 0;
  // The WAL index is in the machine's own byte order.
  return endianness() === "LE" ? index.readUInt32LE(BACKFILL_OFFSET) : index.readUInt32BE(BACKFILL_OFFSET);
}

/** The commits a power cut can cut the log at: each one at or after the frames a checkpoint copied into the file. */
function cuttableCommits(databasePath: string): number[] {
  const log = readFileSync(`${databasePath}-wal`);
  const frameBytes = FRAME_HEADER_BYTES + log.readUInt32BE(8);
  const backfilled = backfilledFrames(databasePath);
  return walCommits(databasePath).filter(length => (length - WAL_HEADER_BYTES) / frameBytes >= backfilled);
}

/** What `read` finds in the database as recovery gives it back with its log cut at `length`, read from a copy. */
export function stateAt<T>(databasePath: string, length: number, read: (database: Database.Database) => T): T {
  const folder = mkdtempSync(join(tmpdir(), "rerail-power-cut-"));
  try {
    const copy = join(folder, "rerail.sqlite");
    copyFileSync(databasePath, copy);
    writeFileSync(`${copy}-wal`, readFileSync(`${databasePath}-wal`).subarray(0, length));
    const database = new Database(copy);
    try {
      return read(database);
    } finally {
      database.close();
    }
  } finally {
    removeFolderSync(folder);
  }
}

/** The newest commit a power cut can cut the log at where `holds` is true of the database; throws when none is. */
export function latestCommitWhere(databasePath: string, holds: (database: Database.Database) => boolean): number {
  const found = cuttableCommits(databasePath).reverse().find(length => stateAt(databasePath, length, holds));
  if (found === undefined) throw new Error(`no commit in the log of ${databasePath} that a power cut can keep holds the state asked for`);
  return found;
}

/**
 * The power cut itself: the log keeps its first `length` bytes, a commit's end (`walCommits`), and the WAL index goes,
 * so the next open rebuilds it from the log as it does after a restart. Each of `lostLines` loses its last lines, as a
 * file the operating system had not written back whole: the session's ledger may keep lines whose rows are gone, and
 * may also lack its own last lines.
 */
export function cutPower(databasePath: string, length: number, lostLines: Readonly<Record<string, number>> = {}): void {
  if (!cuttableCommits(databasePath).includes(length)) {
    throw new Error(`${length} is not the end of a commit a power cut can keep in the log of ${databasePath}`);
  }
  truncateSync(`${databasePath}-wal`, length);
  rmSync(`${databasePath}-shm`, { force: true });
  for (const [path, lost] of Object.entries(lostLines)) {
    const lines = readFileSync(path, "utf8").split("\n").filter(Boolean);
    writeFileSync(path, lines.slice(0, Math.max(0, lines.length - lost)).map(line => `${line}\n`).join(""));
  }
}
