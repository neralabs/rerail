import { constants, existsSync, lstatSync, writeFileSync } from "node:fs";
import { type FileHandle, open, opendir } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
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
import { readLedgerFile, writeLedgerFile } from "../engine/storage/ledger-paths.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { preUpdateSnapshotPath } from "./project-cli.js";
import { scrubbedLedgerLine } from "../runtime/runtime.js";

/** Rows read and rewritten per transaction: a running daemon waits on at most one batch. */
const BATCH_ROWS = 500;
/** How often a ledger that changed while it was scrubbed is read again before it is left for the next run. */
const LEDGER_ATTEMPTS = 3;
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
}

/** What `rerail scrub` rewrote in the session ledgers, and the ledgers it left. */
interface LedgerReport {
  lines: number;
  files: number;
  checkouts: number;
  skipped: string[];
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
 * text cannot be read back from free pages or the WAL.
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
  return { events, audits, auditPages, errors, erased: eraseFreedPages(database) };
}

/** A real folder at `path`, never a link to one. */
function realDirectory(path: string): boolean {
  try {
    return lstatSync(path).isDirectory();
  } catch {
    return false;
  }
}

/**
 * Rewrites one session ledger's lines through `scrubbedLedgerLine`, replacing the file whole (`writeLedgerFile`, which
 * never follows a link). A ledger a session appended to while it was scrubbed is read again; the number of lines
 * changed, or undefined when it kept changing.
 */
async function scrubLedger(path: string, directory: string): Promise<number | undefined> {
  for (let attempt = 0; attempt < LEDGER_ATTEMPTS; attempt += 1) {
    const text = await readLedgerFile(path, directory);
    if (text === undefined) return 0;
    let changed = 0;
    const lines = text.split("\n").map(line => {
      const scrubbed = line === "" ? undefined : scrubbedLedgerLine(line);
      if (scrubbed === undefined) return line;
      changed += 1;
      return scrubbed;
    });
    if (changed === 0) return 0;
    if (await readLedgerFile(path, directory) !== text) continue;
    await writeLedgerFile(path, directory, lines.join("\n"));
    return changed;
  }
  return undefined;
}

/**
 * Scrubs the session ledgers (`<checkout>/.rerail/sessions/<session>/events.jsonl`) of every checkout the database
 * records. A checkout, `.rerail`, `sessions` or session folder that is a link, or not a folder, is skipped, never
 * followed.
 */
async function scrubLedgers(checkoutPaths: readonly string[]): Promise<LedgerReport> {
  const report: LedgerReport = { lines: 0, files: 0, checkouts: 0, skipped: [] };
  for (const checkout of checkoutPaths) {
    const sessions = join(checkout, ".rerail", "sessions");
    if (!isAbsolute(checkout) || !realDirectory(checkout) || !realDirectory(join(checkout, ".rerail")) || !realDirectory(sessions)) continue;
    report.checkouts += 1;
    for await (const entry of await opendir(sessions)) {
      if (!entry.isDirectory()) continue;
      const directory = join(sessions, entry.name);
      const path = join(directory, "events.jsonl");
      try {
        const lines = await scrubLedger(path, directory);
        if (lines === undefined) report.skipped.push(`${path} (a session kept writing to it)`);
        else if (lines > 0) {
          report.lines += lines;
          report.files += 1;
        }
      } catch (error) {
        report.skipped.push(`${path} (${error instanceof Error ? error.message : String(error)})`);
      }
    }
  }
  return report;
}

/**
 * Scrubs `daemon.log` in the Rerail home in place, line by line: the daemon keeps it open to append to, so it is
 * rewritten through a descriptor that follows no link, not replaced. The number of lines changed.
 */
async function scrubDaemonLog(home: string): Promise<number> {
  let handle: FileHandle;
  try {
    handle = await open(join(home, "daemon.log"), constants.O_RDWR | constants.O_NOFOLLOW);
  } catch {
    return 0;
  }
  try {
    if (!(await handle.stat()).isFile()) return 0;
    const text = await handle.readFile("utf8");
    let changed = 0;
    const scrubbed = text.split("\n").map(line => {
      const clean = scrubSecrets(line);
      if (clean !== line) changed += 1;
      return clean;
    }).join("\n");
    if (changed === 0) return 0;
    const bytes = Buffer.from(scrubbed, "utf8");
    await handle.write(bytes, 0, bytes.length, 0);
    await handle.truncate(bytes.length);
    return changed;
  } finally {
    await handle.close();
  }
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

function printDatabaseReport(path: string, report: DatabaseReport): void {
  process.stdout.write(
    `Scrubbed ${path}: ${report.events.scrubbed} of ${plural(report.events.read, "event")}, `
    + `${report.audits.scrubbed} of ${plural(report.audits.read, "audit")}, `
    + `${report.auditPages.scrubbed} of ${plural(report.auditPages.read, "audit result page")}, `
    + `${plural(report.errors, "error message")}.\n`,
  );
  if (!report.erased) {
    process.stdout.write(`Another Rerail process was reading ${path}, so the replaced text may remain in ${path}-wal until it stops; run rerail scrub again then.\n`);
  }
}

/**
 * `rerail scrub`: redacts secrets from what earlier builds stored, with the scrubber writes now use (`secret-scrub`):
 * the Rerail database's events and audit text (then rewritten so the old text is gone from its bytes and WAL), the
 * copy `rerail update` keeps of it for a rollback, the session ledgers of every checkout it records, and `daemon.log`.
 * Running it again changes nothing.
 */
export async function runScrubCommand(args: readonly string[], usage: () => string): Promise<number> {
  parseArguments(args, { flags: [], options: [] }, argument => `Unknown scrub option: ${argument}\n\n${usage()}`);
  const home = defaultRerailHome();
  const path = join(home, RERAIL_DATABASE_FILE);
  markHintShown(home);
  let checkouts: string[] = [];
  if (existsSync(path)) {
    const db = openRerailDatabase(path);
    try {
      db.$client.pragma("busy_timeout = 10000");
      printDatabaseReport(path, scrubDatabase(db));
      checkouts = (db.$client.prepare("SELECT checkout_path AS path FROM checkouts ORDER BY checkout_path").all() as Array<{ path: string }>)
        .map(row => row.path);
    } finally {
      db.$client.close();
    }
  } else process.stdout.write(`No Rerail database at ${path}; no stored history to scrub.\n`);
  // The copy `rerail update` took before switching versions, which `rerail update rollback` puts back: opened without
  // this build's schema, so it stays the earlier version's file.
  const copy = preUpdateSnapshotPath(path);
  if (existsSync(copy)) {
    try {
      const db = openSqliteDatabase(copy);
      try {
        printDatabaseReport(copy, scrubDatabase(db));
      } finally {
        db.$client.close();
      }
    } catch (error) {
      process.stdout.write(`Could not scrub ${copy} (${error instanceof Error ? error.message : String(error)}); it may still hold secrets.\n`);
    }
  }
  const ledgers = await scrubLedgers(checkouts);
  const daemonLogLines = await scrubDaemonLog(home);
  process.stdout.write(
    `Scrubbed ${plural(ledgers.lines, "line")} in ${plural(ledgers.files, "session ledger")} across ${plural(ledgers.checkouts, "checkout")}; `
    + `${plural(daemonLogLines, "line")} of daemon.log.\n`,
  );
  for (const skipped of ledgers.skipped) process.stdout.write(`Left ${skipped}; run rerail scrub again.\n`);
  return 0;
}
