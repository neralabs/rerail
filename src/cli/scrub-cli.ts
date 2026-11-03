import { constants, existsSync, lstatSync, writeFileSync } from "node:fs";
import { type FileHandle, open, opendir } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { deserialize, serialize } from "node:v8";
import { parseArguments } from "../daemon/daemon.js";
import { storedFailures, storedViolations } from "../engine/audit/audit-store.js";
import type { AuditFailure } from "../engine/audit/audit.js";
import type { CodeRuleViolation } from "../engine/code-rule/code-rule.js";
import { scrubSecrets, scrubValue } from "../engine/secret-scrub.js";
import { stableJsonText } from "../engine/stable-json.js";
import {
  eraseFreedPages, openRerailDatabase, openSqliteDatabase, RERAIL_DATABASE_FILE, type RerailDatabase,
} from "../engine/storage/database.js";
import { openLedgerFile, readLedgerFile, writeLedgerFile } from "../engine/storage/ledger-paths.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { type HookHost, scrubbedPendingText, withPendingFileLock } from "../harness/hook-adapter.js";
import { preUpdateSnapshotPath } from "./project-cli.js";
import { DAEMON_LOG_FILE } from "../runtime/daemon-control.js";
import { scrubbedLedgerLine } from "../runtime/runtime.js";

/** Rows read and rewritten per transaction: a running daemon waits on at most one batch. */
const BATCH_ROWS = 500;
/** Passes `rewriteAppendedFile` makes over a file before it is left for the next run. */
const REWRITE_ATTEMPTS = 5;
/** How long `rewriteAppendedFile` waits for a line being appended to end before it reads the file again. */
const MID_LINE_WAIT_MS = 50;
/**
 * Opens the line `rewriteAppendedFile` appends to reserve room, padded with spaces to the size it needs. A session
 * ledger reader skips it (it is JSON, and no ledger entry); `rewriteAppendedFile` drops it, and one a crashed run left.
 */
const RESERVE_LINE_START = '{"rerail-scrub-reserve":"';
const RESERVE_LINE_END = '"}\n';
/** The hosts whose hooks hold steers between hooks (`<storageRoot>/<host>-pending`). */
const PENDING_HOSTS = Object.keys({ "claude-code": true, codex: true, cursor: true } satisfies Record<HookHost, true>);
/** Written once the hint to run `rerail scrub` was shown (or had nothing to scrub), so it is shown once. */
const SCRUB_HINT_FILE = "scrub-hint.json";

/** What `rerail scrub` rewrote in one database: rows changed, of those it read. */
interface DatabaseReport {
  events: { scrubbed: number; read: number };
  audits: { scrubbed: number; read: number };
  auditPages: { scrubbed: number; read: number };
  errors: number;
  /** False when another connection's read kept the WAL from being emptied (`eraseFreedPages`). */
  erased: boolean;
  /** Why `eraseFreedPages` failed (busy, or no disk space for `VACUUM`'s copy); the rows are rewritten all the same. */
  eraseFailure?: string;
}

/** What `rerail scrub` rewrote below the checkouts' session folders, and the files it left. */
interface SessionFilesReport {
  lines: number;
  files: number;
  pendingFiles: number;
  checkouts: number;
  skipped: string[];
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** A stored event payload as `persistEvent` would now write it; undefined when it holds no secret. */
function scrubbedPayload(json: string, v8: Buffer | null): { json: string; v8: Buffer | null } | undefined {
  const payload: unknown = v8 === null ? JSON.parse(json) : deserialize(v8);
  const scrubbed = scrubValue(payload);
  if (scrubbed === payload) return undefined;
  return { json: stableJsonText(scrubbed) ?? JSON.stringify({ encoding: "node:v8" }), v8: v8 === null ? null : serialize(scrubbed) };
}

/** Findings JSON as the audit store now writes it; undefined when unchanged. */
function scrubbedFindings(violationsJson: string, failuresJson: string): { violations: string; failures: string } | undefined {
  const violationsBefore = JSON.parse(violationsJson) as CodeRuleViolation[];
  const failuresBefore = JSON.parse(failuresJson) as AuditFailure[];
  const violations = storedViolations(violationsBefore);
  const failures = storedFailures(failuresBefore);
  const changed = violations.some((violation, index) => violation !== violationsBefore[index])
    || failures.some((failure, index) => failure !== failuresBefore[index]);
  return changed ? { violations: JSON.stringify(violations), failures: JSON.stringify(failures) } : undefined;
}

/**
 * Rewrites one table's rows in batches by `key`, ascending from `start` (its nulls select from the first row), each
 * batch in its own immediate transaction. `rewrite` gives a row's new values, or undefined to leave it. A file without
 * the table or one of its `columns` (`[table, ...columns]`), as an older build's copy may be, is left as it is.
 */
function rewriteRows<Row extends Record<string, unknown>>(
  database: RerailDatabase,
  query: { columns: readonly [string, ...string[]]; select: string; update: string; start: readonly null[] },
  key: (row: Row) => unknown[],
  rewrite: (row: Row) => unknown[] | undefined,
): { scrubbed: number; read: number } {
  const client = database.$client;
  const [table, ...columns] = query.columns;
  const present = new Set((client.pragma(`table_info(${table})`) as Array<{ name: string }>).map(column => column.name));
  if (!columns.every(column => present.has(column))) return { scrubbed: 0, read: 0 };
  const next = client.prepare(query.select);
  const write = client.prepare(query.update);
  let after: readonly unknown[] = query.start;
  let scrubbed = 0;
  let read = 0;
  for (;;) {
    const rows = next.all(...after, BATCH_ROWS) as Row[];
    if (rows.length === 0) break;
    client.transaction(() => {
      for (const row of rows) {
        const values = rewrite(row);
        if (values === undefined) continue;
        write.run(...values, ...key(row));
        scrubbed += 1;
      }
    }).immediate();
    read += rows.length;
    after = key(rows.at(-1)!);
  }
  return { scrubbed, read };
}

/**
 * Rewrites the Rerail database's stored text through the scrubber the writers now use: event payloads, audit findings
 * and pages, and audit error text. Then `eraseFreedPages` on the same connection rewrites the file, so the replaced
 * text cannot be read back from free pages or the WAL; its failure is reported, not thrown, since the rows are done.
 */
function scrubDatabase(database: RerailDatabase): DatabaseReport {
  type EventRow = { sequence: number; payloadJson: string; payloadV8: Buffer | null };
  const events = rewriteRows<EventRow>(database, {
    columns: ["events", "sequence", "payload_json", "payload_v8"],
    select: "SELECT sequence, payload_json AS payloadJson, payload_v8 AS payloadV8 FROM events "
      + "WHERE sequence > coalesce(?, -1) ORDER BY sequence LIMIT ?",
    update: "UPDATE events SET payload_json = ?, payload_v8 = ? WHERE sequence = ?",
    start: [null],
  }, row => [row.sequence], row => {
    const payload = scrubbedPayload(row.payloadJson, row.payloadV8);
    return payload === undefined ? undefined : [payload.json, payload.v8];
  });
  type AuditRow = { auditId: string; violations: string; failures: string };
  const findings = (row: AuditRow): unknown[] | undefined => {
    const scrubbed = scrubbedFindings(row.violations, row.failures);
    return scrubbed === undefined ? undefined : [scrubbed.violations, scrubbed.failures];
  };
  const audits = rewriteRows<AuditRow>(database, {
    columns: ["audits", "audit_id", "violations_json", "failures_json"],
    select: "SELECT audit_id AS auditId, violations_json AS violations, failures_json AS failures FROM audits "
      + "WHERE audit_id > coalesce(?, '') ORDER BY audit_id LIMIT ?",
    update: "UPDATE audits SET violations_json = ?, failures_json = ? WHERE audit_id = ?",
    start: [null],
  }, row => [row.auditId], findings);
  const auditPages = rewriteRows<AuditRow & { pageIndex: number }>(database, {
    columns: ["audit_result_pages", "audit_id", "page_index", "violations_json", "failures_json"],
    select: "SELECT audit_id AS auditId, page_index AS pageIndex, violations_json AS violations, failures_json AS failures "
      + "FROM audit_result_pages WHERE (audit_id, page_index) > (coalesce(?, ''), coalesce(?, -1)) ORDER BY audit_id, page_index LIMIT ?",
    update: "UPDATE audit_result_pages SET violations_json = ?, failures_json = ? WHERE audit_id = ? AND page_index = ?",
    start: [null, null],
  }, row => [row.auditId, row.pageIndex], findings);
  let errors = 0;
  for (const [table, key] of [["repository_audits", "workspace_root"], ["audit_requests", "id"]] as const) {
    errors += rewriteRows<{ id: string; error: string }>(database, {
      columns: [table, key, "error"],
      select: `SELECT ${key} AS id, error FROM ${table} WHERE error IS NOT NULL AND ${key} > coalesce(?, '') ORDER BY ${key} LIMIT ?`,
      update: `UPDATE ${table} SET error = ? WHERE ${key} = ?`,
      start: [null],
    }, row => [row.id], row => {
      const error = scrubSecrets(row.error);
      return error === row.error ? undefined : [error];
    }).scrubbed;
  }
  let eraseFailure: string | undefined;
  let erased = false;
  try {
    erased = eraseFreedPages(database);
  } catch (error) {
    eraseFailure = message(error);
  }
  return { events, audits, auditPages, errors, erased, ...(eraseFailure === undefined ? {} : { eraseFailure }) };
}

/** A real folder at `path`, never a link to one. */
function realDirectory(path: string): boolean {
  try {
    return lstatSync(path).isDirectory();
  } catch {
    return false;
  }
}

/** The whole file, read by position from its start, so passes over one descriptor never depend on its offset. */
async function readWhole(handle: FileHandle): Promise<Buffer> {
  const chunks: Buffer[] = [];
  let position = 0;
  for (;;) {
    const chunk = Buffer.allocUnsafe(Math.max(64 * 1024, (await handle.stat()).size - position));
    const { bytesRead } = await handle.read(chunk, 0, chunk.length, position);
    if (bytesRead === 0) return Buffer.concat(chunks);
    chunks.push(chunk.subarray(0, bytesRead));
    position += bytesRead;
  }
}

async function writeAt(handle: FileHandle, bytes: Buffer, position: number): Promise<void> {
  for (let written = 0; written < bytes.length;) {
    written += (await handle.write(bytes, written, bytes.length - written, position + written)).bytesWritten;
  }
}

function reserveLine(bytes: number): string {
  return `${RESERVE_LINE_START}${" ".repeat(Math.max(0, bytes - RESERVE_LINE_START.length - RESERVE_LINE_END.length))}${RESERVE_LINE_END}`;
}

/**
 * Rewrites the complete lines of a file other processes append to through `rewriteLine` (the new line, or undefined
 * to keep it), and never loses or overwrites a line they append meanwhile. Every process that writes the files
 * `rerail scrub` rewrites this way opens them with `O_APPEND` and writes a line per write: the session runtime its
 * ledger lines (in the daemon or in a coding agent's own process), the daemon and the launcher that starts it their
 * output to `daemon.log`. What they append lands past the end of the file as last read here; only bytes before that
 * end are written, and the file never shrinks, so no append is cut off or written over.
 *
 * Lines that come out no longer than they were are written over in place, the last changed one padded at its end with
 * spaces (JSON and log readers ignore them). Longer ones need room first: a reserve line is appended, which lands after
 * whatever was appended since, and the next pass rewrites everything up to and including it, moving the lines between
 * along and dropping it. A reserve line is only appended after a complete line, so it never joins one cut short.
 * `openFile` opens the file with the given flags without following a link. The number of lines changed, or undefined
 * when the file was left: it kept ending mid-line where that line would change, or it was replaced while scrubbed.
 */
export async function rewriteAppendedFile(
  openFile: (flags: number) => Promise<FileHandle>,
  rewriteLine: (line: string) => string | undefined,
): Promise<number | undefined> {
  let handle: FileHandle;
  try {
    handle = await openFile(constants.O_RDWR);
  } catch (error) {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") return 0;
    throw error;
  }
  let appender: FileHandle | undefined;
  try {
    const stats = await handle.stat();
    if (!stats.isFile()) return 0;
    for (let attempt = 0; attempt < REWRITE_ATTEMPTS; attempt += 1) {
      const bytes = await readWhole(handle);
      const end = bytes.lastIndexOf(0x0a) + 1;
      const tail = bytes.subarray(end).toString("utf8");
      const midLine = tail !== "" && rewriteLine(tail) !== undefined;
      // Lines as bytes: one kept is written back exactly as it was, even where it is not valid UTF-8.
      const lines: Buffer[] = [];
      let changed = 0;
      let lastChanged = -1;
      let reserves = 0;
      for (let start = 0; start < end;) {
        const stop = bytes.indexOf(0x0a, start);
        const line = bytes.subarray(start, stop);
        start = stop + 1;
        const text = line.toString("utf8");
        if (text.startsWith(RESERVE_LINE_START)) {
          reserves += 1;
          continue;
        }
        const rewritten = rewriteLine(text);
        if (rewritten !== undefined) {
          changed += 1;
          lastChanged = lines.length;
        }
        lines.push(rewritten === undefined ? line : Buffer.from(rewritten, "utf8"));
      }
      if (midLine && attempt < REWRITE_ATTEMPTS - 1) {
        await delay(MID_LINE_WAIT_MS);
        continue;
      }
      // A file of nothing but reserve lines keeps them: dropping them would leave only padding, which is no JSON.
      if (changed === 0 && (reserves === 0 || lines.length === 0)) return midLine ? undefined : 0;
      const size = lines.reduce((total, line) => total + line.length + 1, 0);
      if (size <= end) {
        const padded = lastChanged === -1 ? lines.length - 1 : lastChanged;
        const newline = Buffer.from("\n");
        await writeAt(handle, Buffer.concat(lines.flatMap((line, index) => (
          index === padded ? [line, Buffer.from(`${" ".repeat(end - size)}\n`)] : [line, newline]
        ))), 0);
        return midLine ? undefined : changed;
      }
      if (tail !== "") {
        await delay(MID_LINE_WAIT_MS);
        continue;
      }
      if (appender === undefined) {
        appender = await openFile(constants.O_WRONLY | constants.O_APPEND);
        const appended = await appender.stat();
        if (appended.ino !== stats.ino || appended.dev !== stats.dev) return undefined;
      }
      await appender.write(reserveLine(size - end), null, "utf8");
    }
    return undefined;
  } finally {
    await appender?.close();
    await handle.close();
  }
}

/** One session ledger line as it is now written (`scrubbedLedgerLine`); undefined when it stays. */
function ledgerLine(line: string): string | undefined {
  return line === "" ? undefined : scrubbedLedgerLine(line);
}

/** One `daemon.log` line as the daemon now writes it (`scrubWrites`); undefined when it stays. */
function logLine(line: string): string | undefined {
  const scrubbed = scrubSecrets(line);
  return scrubbed === line ? undefined : scrubbed;
}

/**
 * Scrubs the steers hooks hold between hooks (`<sessions>/<host>-pending/<session>.json`). A hook reads and replaces
 * one only while it holds that session's lock, and so does this (`withPendingFileLock`), so neither replaces a steer
 * the other wrote. A folder or file that is a link is skipped, never followed.
 */
async function scrubPending(sessions: string, report: SessionFilesReport): Promise<void> {
  for (const host of PENDING_HOSTS) {
    const folder = join(sessions, `${host}-pending`);
    if (!realDirectory(folder)) continue;
    for await (const entry of await opendir(folder)) {
      if (!entry.isFile() || !entry.name.endsWith(".json")) continue;
      const path = join(folder, entry.name);
      try {
        const scrubbed = await withPendingFileLock(path, async () => {
          const text = await readLedgerFile(path, folder);
          const next = text === undefined ? undefined : scrubbedPendingText(text);
          if (next !== undefined) await writeLedgerFile(path, folder, next);
          return next !== undefined;
        });
        if (scrubbed) report.pendingFiles += 1;
      } catch (error) {
        report.skipped.push(`${path} (${message(error)})`);
      }
    }
  }
}

/**
 * Scrubs the session ledgers (`<checkout>/.rerail/sessions/<session>/events.jsonl`, `rewriteAppendedFile`) and held
 * steers (`scrubPending`) of every checkout the database records. A checkout, `.rerail`, `sessions` or session folder
 * that is a link, or not a folder, is skipped, never followed; so is a ledger that is a link.
 */
async function scrubSessionFiles(checkoutPaths: readonly string[]): Promise<SessionFilesReport> {
  const report: SessionFilesReport = { lines: 0, files: 0, pendingFiles: 0, checkouts: 0, skipped: [] };
  for (const checkout of checkoutPaths) {
    const sessions = join(checkout, ".rerail", "sessions");
    if (!isAbsolute(checkout) || !realDirectory(checkout) || !realDirectory(join(checkout, ".rerail")) || !realDirectory(sessions)) continue;
    report.checkouts += 1;
    for await (const entry of await opendir(sessions)) {
      if (!entry.isDirectory()) continue;
      const path = join(sessions, entry.name, "events.jsonl");
      try {
        const lines = await rewriteAppendedFile(flags => openLedgerFile(path, flags), ledgerLine);
        if (lines === undefined) report.skipped.push(`${path} (a session kept writing to it)`);
        else if (lines > 0) {
          report.lines += lines;
          report.files += 1;
        }
      } catch (error) {
        report.skipped.push(`${path} (${message(error)})`);
      }
    }
    await scrubPending(sessions, report);
  }
  return report;
}

/** Scrubs `daemon.log` in the Rerail home in place (`rewriteAppendedFile`), never through a link. */
async function scrubDaemonLog(home: string, report: SessionFilesReport): Promise<number> {
  const path = join(home, DAEMON_LOG_FILE);
  try {
    const lines = await rewriteAppendedFile(flags => open(path, flags | constants.O_NOFOLLOW), logLine);
    if (lines !== undefined) return lines;
    report.skipped.push(`${path} (Rerail kept writing to it)`);
  } catch (error) {
    report.skipped.push(`${path} (${message(error)})`);
  }
  return 0;
}

/** Marks the post-update hint shown; a hint file already there is kept. */
function markHintShown(home: string): void {
  try {
    writeFileSync(join(home, SCRUB_HINT_FILE), `${JSON.stringify({ shownAt: Date.now() })}\n`, { mode: 0o600, flag: "wx" });
  } catch {
    // Already marked, or the home is not writable: the hint shows again next time, which is harmless.
  }
}

/**
 * The one-time hint, after an update to a build that scrubs what it writes, to scrub what earlier builds stored:
 * the text to print, or undefined once shown (`SCRUB_HINT_FILE`) or where there is no database to scrub.
 */
export function scrubHintOnce(home: string): string | undefined {
  if (existsSync(join(home, SCRUB_HINT_FILE))) return undefined;
  markHintShown(home);
  if (!existsSync(join(home, RERAIL_DATABASE_FILE))) return undefined;
  return "Rerail now redacts secrets (API keys, tokens, passwords, private keys) from the history it stores. "
    + "Run `rerail scrub` once to redact them from history recorded before this update.\n";
}

function plural(count: number, noun: string): string {
  return `${count} ${count === 1 ? noun : `${noun}s`}`;
}

/** Prints what `scrubDatabase` did to the file at `path`; false when some of it is left for another run. */
function printDatabaseReport(path: string, report: DatabaseReport): boolean {
  process.stdout.write(
    `Scrubbed ${path}: ${report.events.scrubbed} of ${plural(report.events.read, "event")}, `
    + `${report.audits.scrubbed} of ${plural(report.audits.read, "audit")}, `
    + `${report.auditPages.scrubbed} of ${plural(report.auditPages.read, "audit result page")}, `
    + `${plural(report.errors, "error message")}.\n`,
  );
  if (report.eraseFailure !== undefined) {
    process.stdout.write(
      `Could not rewrite ${path} to erase the replaced text (${report.eraseFailure}). The rows are redacted, but their `
      + `old text may remain in ${path}-wal or the file's free pages. Run rerail scrub again when Rerail is idle, with `
      + "free disk space for a second copy of the database.\n",
    );
    return false;
  }
  if (!report.erased) {
    process.stdout.write(`Another Rerail process was reading ${path}, so the replaced text may remain in ${path}-wal until it stops; run rerail scrub again then.\n`);
    return false;
  }
  return true;
}

/** Scrubs the database at `path` as `scrubDatabase` does, then prints the report; false when something is left. */
function scrubDatabaseFile(path: string, openDatabase: () => RerailDatabase, opened?: (database: RerailDatabase) => void): boolean {
  try {
    const db = openDatabase();
    try {
      db.$client.pragma("busy_timeout = 10000");
      opened?.(db);
      return printDatabaseReport(path, scrubDatabase(db));
    } finally {
      db.$client.close();
    }
  } catch (error) {
    process.stdout.write(`Could not scrub ${path} (${message(error)}); it may still hold secrets. Run rerail scrub again when Rerail is idle.\n`);
    return false;
  }
}

/**
 * `rerail scrub`: redacts secrets from what earlier builds stored, with the scrubber writes now use (`secret-scrub`):
 * the Rerail database's events and audit text (then rewritten so the old text is gone from its bytes and WAL), the
 * copy `rerail update` keeps of it for a rollback, the session ledgers and held steers of every checkout it records,
 * and `daemon.log`. A running daemon and running sessions carry on: no line a session or the daemon appends to a
 * ledger or `daemon.log` while it runs is lost or written over (`rewriteAppendedFile`), and a hook never loses a steer
 * it holds (`scrubPending`). A step that fails is reported and the others still run; the exit code is 1 when anything
 * was left for another run. Running it again changes nothing.
 */
export async function runScrubCommand(args: readonly string[], usage: () => string): Promise<number> {
  parseArguments(args, { flags: [], options: [] }, argument => `Unknown scrub option: ${argument}\n\n${usage()}`);
  const home = defaultRerailHome();
  const path = join(home, RERAIL_DATABASE_FILE);
  markHintShown(home);
  let complete = true;
  let checkouts: string[] = [];
  if (existsSync(path)) {
    // The checkouts are read first, so the ledgers are scrubbed even when the database cannot be.
    complete = scrubDatabaseFile(path, () => openRerailDatabase(path), db => {
      checkouts = (db.$client.prepare("SELECT checkout_path AS path FROM checkouts ORDER BY checkout_path").all() as Array<{ path: string }>)
        .map(row => row.path);
    });
  } else process.stdout.write(`No Rerail database at ${path}; no stored history to scrub.\n`);
  // The copy `rerail update` took before switching versions: opened without this build's schema, so it stays the
  // earlier version's file.
  const copy = preUpdateSnapshotPath(path);
  if (existsSync(copy)) complete = scrubDatabaseFile(copy, () => openSqliteDatabase(copy)) && complete;
  const files = await scrubSessionFiles(checkouts);
  const daemonLogLines = await scrubDaemonLog(home, files);
  process.stdout.write(
    `Scrubbed ${plural(files.lines, "line")} in ${plural(files.files, "session ledger")} and ${plural(files.pendingFiles, "held steer file")} `
    + `across ${plural(files.checkouts, "checkout")}; ${plural(daemonLogLines, "line")} of daemon.log.\n`,
  );
  for (const skipped of files.skipped) process.stdout.write(`Left ${skipped}; run rerail scrub again.\n`);
  return complete && files.skipped.length === 0 ? 0 : 1;
}
