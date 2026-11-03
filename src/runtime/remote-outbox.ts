/**
 * Internal testing: the remote sink's outbox (docs/remote-sink.md). It reads the Rerail database and sends what a
 * remote has not acknowledged as NDJSON batches (`POST <url>/v1/batches`), one remote at a time:
 *
 * - Streams: `repository`, `rule`, `session`, `event`, `audit` (with its `violation`s), `scan`, and `gap`.
 * - Events go by `sequence`, a cursor per remote that advances only when the remote answers 200. A hole in the
 *   sequence (history reset or deleted) is sent as a `gap`.
 * - Every other stream is a snapshot: a row is sent again when its content hash changes (a session that ends, an
 *   audit re-recorded).
 * - Batches are at most {@link MAX_BATCH_BYTES} uncompressed; a record larger than that is never sent, only a `gap`.
 * - 5xx, timeouts and network errors retry with exponential backoff; 400 and 413 skip the batch and send a `gap` for
 *   it; 401 and 403 wait for the configuration to change (or the longest backoff).
 * - Nothing here blocks a session: the daemon runs it on a timer, on its own connections, and catches every error.
 *
 * The outbox's own state (cursors, sent hashes, retry state, queued headless scans) is `<home>/remote-outbox.sqlite`,
 * apart from the Rerail database, so removing every remote or the file loses nothing else.
 */
import { createHash, randomUUID } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import Database from "better-sqlite3";
import type { AuditRecord, AuditRuleProvenance } from "../engine/audit/audit.js";
import { SqliteAuditReader } from "../engine/audit/audit-store.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { parseRuleDocument } from "../engine/catalog/rule-documents.js";
import { worktreeRulesDirectory } from "../engine/catalog/worktree-pack.js";
import type { RuleDocument } from "../engine/catalog/rule-store.js";
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { RERAIL_DATABASE_FILE } from "../engine/storage/database.js";
import { keepFilesPrivate } from "../engine/storage/private-files.js";
import { defaultRerailHome, REMOTE_OUTBOX_FILE } from "../engine/storage/rerail-home.js";
import { ruleDocumentDigest } from "../engine/stable-json.js";
import {
  excludedAt, machineId, machineLabel, readRemoteConfig, readRemoteToken, remoteStateKey, type RemoteEntry, type RepositoryExclusion,
} from "./remote-config.js";

export const SINK_PROTOCOL = 1;
/** C1: one batch is at most 5 MB uncompressed. */
export const MAX_BATCH_BYTES = 5 * 1024 * 1024;
const MAX_EVENTS_PER_READ = 2_000;
const REQUEST_TIMEOUT_MS = 30_000;
const MIN_BACKOFF_MS = 2_000;
export const MAX_BACKOFF_MS = 5 * 60_000;
const LOCK_TTL_MS = 2 * 60_000;
/** Records one sync may skip because the remote refused each alone (400/413) before it backs off instead. */
const MAX_SKIPPED_RECORDS_PER_SYNC = 3;
/** `events.payload_json` of a payload only `node:v8` could store: it has no JSON form to send. */
const V8_ONLY_PAYLOAD = JSON.stringify({ encoding: "node:v8" });

type Json = Record<string, unknown>;

/** One NDJSON line: a record with its stream (`type`) and stable `id`, and what sending it acknowledges. */
interface OutboxRecord {
  readonly line: string;
  readonly stream: string;
  readonly id: string;
  /** Snapshot streams: the content hash the remote holds once this is acknowledged. */
  readonly hash?: string;
  /** Events: the sequence the cursor moves to once this is acknowledged. */
  readonly sequence?: number;
}

export interface RemoteStatus {
  readonly name: string;
  readonly url: string;
  readonly content: boolean;
  readonly lastAttemptAt: number | null;
  readonly lastSuccessAt: number | null;
  readonly lastError: string | null;
  readonly consecutiveFailures: number;
  readonly nextAttemptAt: number | null;
  readonly sentBatches: number;
  readonly sentRecords: number;
  /** Events recorded locally that this remote has not acknowledged yet. */
  readonly pendingEvents: number;
  readonly queuedScans: number;
}

export interface SyncResult {
  readonly remote: string;
  readonly batches: number;
  readonly records: number;
  /** Set when the last attempt failed; the remote is retried after `nextAttemptAt`. */
  readonly error?: string;
  /** Nothing was attempted: another process holds this remote's lock, or its backoff has not passed. */
  readonly skipped?: "locked" | "backoff" | "no-token" | "removed";
}

export interface SyncOptions {
  /** Ignore the backoff (`rerail remote push`). */
  readonly force?: boolean;
  readonly maxBatches?: number;
  readonly batchBytes?: number;
  /** Stops between batches and aborts a request in flight (the daemon closing). */
  readonly signal?: AbortSignal;
}

export interface OutboxOptions {
  readonly home?: string;
  readonly fetch?: typeof fetch;
  readonly now?: () => number;
}

const hashOf = (value: unknown): string => createHash("sha256").update(JSON.stringify(value)).digest("hex");
const record = (stream: string, id: string, fields: Json, extra: Pick<OutboxRecord, "hash" | "sequence"> = {}): OutboxRecord => ({
  line: JSON.stringify({ type: stream, id, ...fields }),
  stream,
  id,
  ...extra,
});
const parse = (text: string | null): unknown => {
  if (text === null) return null;
  try { return JSON.parse(text) as unknown; } catch { return text; }
};

function openState(home: string): Database.Database {
  const database = new Database(join(home, REMOTE_OUTBOX_FILE));
  keepFilesPrivate(home, [REMOTE_OUTBOX_FILE]);
  database.pragma("journal_mode = WAL");
  database.pragma("busy_timeout = 5000");
  database.exec(`
    CREATE TABLE IF NOT EXISTS cursors (remote TEXT NOT NULL, stream TEXT NOT NULL, value INTEGER NOT NULL, PRIMARY KEY (remote, stream));
    CREATE TABLE IF NOT EXISTS sent (remote TEXT NOT NULL, stream TEXT NOT NULL, id TEXT NOT NULL, hash TEXT NOT NULL, PRIMARY KEY (remote, stream, id));
    CREATE TABLE IF NOT EXISTS pending_gaps (remote TEXT NOT NULL, id TEXT NOT NULL, record_json TEXT NOT NULL, PRIMARY KEY (remote, id));
    CREATE TABLE IF NOT EXISTS status (
      remote TEXT PRIMARY KEY, last_attempt_at INTEGER, last_success_at INTEGER, last_error TEXT,
      consecutive_failures INTEGER NOT NULL DEFAULT 0, next_attempt_at INTEGER, config_hash TEXT,
      sent_batches INTEGER NOT NULL DEFAULT 0, sent_records INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE IF NOT EXISTS scans (scan_id TEXT PRIMARY KEY, repository_key TEXT NOT NULL, document_json TEXT NOT NULL, rules_json TEXT NOT NULL, queued_at INTEGER NOT NULL);
    CREATE TABLE IF NOT EXISTS locks (remote TEXT PRIMARY KEY, holder TEXT NOT NULL, expires_at INTEGER NOT NULL);
  `);
  return database;
}

/** The rule document an audit ran, by its payload hash: an installed or bundled pack's, a library rule's, or a worktree's. */
class RuleDocuments {
  #catalog: ReturnType<typeof loadRuleCatalog> | undefined;
  readonly #worktrees = new Map<string, Map<string, RuleDocument>>();

  constructor(private readonly rerail: Database.Database | undefined) {}

  find(provenance: Pick<AuditRuleProvenance, "ruleId"> & { payloadHash?: string; checkoutPath?: string }): RuleDocument | undefined {
    const hash = provenance.payloadHash;
    if (hash === undefined) return undefined;
    try {
      this.#catalog ??= loadRuleCatalog();
      const bundled = this.#catalog.documents.get(provenance.ruleId);
      if (bundled !== undefined && ruleDocumentDigest(bundled) === hash) return bundled;
    } catch { /* an unreadable catalog leaves the other sources */ }
    const library = this.rerail?.prepare("SELECT document_json AS json FROM library_rules WHERE rule_id = ? AND payload_hash = ?").get(provenance.ruleId, hash) as { json: string } | undefined;
    if (library !== undefined) {
      try { return parseRuleDocument(JSON.parse(library.json), provenance.ruleId); } catch { /* unreadable: not sent */ }
    }
    if (provenance.checkoutPath !== undefined) {
      const documents = this.#worktree(provenance.checkoutPath);
      const found = documents.get(provenance.ruleId);
      if (found !== undefined && ruleDocumentDigest(found) === hash) return found;
    }
    return undefined;
  }

  #worktree(checkoutPath: string): Map<string, RuleDocument> {
    let documents = this.#worktrees.get(checkoutPath);
    if (documents !== undefined) return documents;
    documents = new Map();
    const folder = worktreeRulesDirectory(checkoutPath);
    try {
      for (const name of readdirSync(folder).filter(entry => entry.endsWith(".json"))) {
        try {
          const document = parseRuleDocument(JSON.parse(readFileSync(join(folder, name), "utf8")), name);
          documents.set(document.rule.id, document);
        } catch { /* a broken file is the worktree's own problem */ }
      }
    } catch { /* no folder */ }
    this.#worktrees.set(checkoutPath, documents);
    return documents;
  }
}

/** Collects one batch of records the remote has not acknowledged, in dependency order, within the byte budget. */
class BatchBuilder {
  readonly records: OutboxRecord[] = [];
  bytes = 0;
  full = false;
  readonly oversized: OutboxRecord[] = [];
  /** `stream\0id` of every record taken, so a rule many audits name goes in once. */
  readonly ids = new Set<string>();

  #sendable = 0;

  /** `maxSendable`: at most this many records that go out (narrowing down a record the remote refuses). */
  constructor(readonly budget: number, readonly maxSendable: number = Number.POSITIVE_INFINITY) {}

  /** False once the batch is full: the caller stops collecting. */
  add(item: OutboxRecord): boolean {
    if (item.line !== "" && this.#sendable >= this.maxSendable) {
      this.full = true;
      return false;
    }
    const size = Buffer.byteLength(item.line) + 1;
    if (size > this.budget) {
      this.oversized.push(item);
      return true;
    }
    if (this.bytes + size > this.budget) {
      this.full = true;
      return false;
    }
    this.records.push(item);
    if (item.line !== "") this.#sendable += 1;
    this.ids.add(`${item.stream}\0${item.id}`);
    this.bytes += size;
    return true;
  }
}

/**
 * Which records are opted out (`rerail remote exclude`), by when they were recorded: a repository's exclusions apply to
 * every key it has had (its aliases: an older clone id, a remote it used to have), and a session with no repository
 * is placed by its folder's recorded checkout. One that cannot be placed is held back while any exclusion is open.
 */
class Exclusions {
  readonly #byKey = new Map<string, RepositoryExclusion[]>();
  readonly #all: readonly RepositoryExclusion[];
  readonly #checkouts: Array<{ path: string; key: string }>;

  constructor(exclusions: readonly RepositoryExclusion[], rerail: Database.Database | undefined) {
    this.#all = exclusions;
    const repositories = rerail === undefined ? [] : rerail.prepare("SELECT repository_key AS key, aliases_json AS aliases FROM repositories").all() as Array<{ key: string; aliases: string }>;
    const names = repositories.map(row => {
      const aliases = parse(row.aliases);
      return new Set([row.key, ...(Array.isArray(aliases) ? aliases.filter((alias): alias is string => typeof alias === "string") : [])]);
    });
    for (const exclusion of exclusions) {
      const keys = new Set([exclusion.repositoryKey]);
      for (const repository of names) if (repository.has(exclusion.repositoryKey)) for (const name of repository) keys.add(name);
      for (const key of keys) this.#byKey.set(key, [...this.#byKey.get(key) ?? [], exclusion]);
    }
    this.#checkouts = exclusions.length === 0 || rerail === undefined ? [] : (rerail.prepare("SELECT checkout_path AS path, repository_key AS key FROM checkouts").all() as Array<{ path: string; key: string }>)
      .sort((left, right) => right.path.length - left.path.length);
  }

  /** The repository of the recorded checkout holding `folder`, the innermost; null when none does. */
  repositoryOfFolder(folder: string): string | null {
    return this.#checkouts.find(checkout => folder === checkout.path || folder.startsWith(`${checkout.path}/`))?.key ?? null;
  }

  /** Whether something of `repositoryKey` (null: not known) recorded at `at` is opted out. */
  at(repositoryKey: string | null, at: number): boolean {
    if (this.#all.length === 0) return false;
    if (repositoryKey === null) return this.#all.some(exclusion => exclusion.from <= at && (exclusion.to === null || at < exclusion.to));
    return (this.#byKey.get(repositoryKey) ?? []).some(exclusion => exclusion.from <= at && (exclusion.to === null || at < exclusion.to));
  }
}

export class RemoteOutbox {
  readonly #home: string;
  readonly #fetch: typeof fetch;
  readonly #now: () => number;
  readonly #holder = randomUUID();

  constructor(options: OutboxOptions = {}) {
    this.#home = options.home ?? defaultRerailHome();
    this.#fetch = options.fetch ?? fetch;
    this.#now = options.now ?? Date.now;
  }

  /** Queues a headless scan report (`rerail.scan/1`) for every remote; false when no remote would receive it. */
  queueScan(
    report: { scanId: string; repository: { key: string }; rules: ReadonlyArray<{ ruleId: string; payloadHash: string }>; checkout: { path: string } },
    /** Rule documents found nowhere else later (a `--rules-ref` scan's), by payload hash; kept with the scan. */
    documents: ReadonlyMap<string, RuleDocument> = new Map(),
  ): boolean {
    const config = readRemoteConfig(this.#home);
    if (config.remotes.length === 0 || excludedAt(config.exclusions, report.repository.key, this.#now())) return false;
    const rules = report.rules.map(rule => ({
      ruleId: rule.ruleId, payloadHash: rule.payloadHash, checkoutPath: report.checkout.path,
      ...(documents.has(rule.payloadHash) ? { document: documents.get(rule.payloadHash) } : {}),
    }));
    const state = openState(this.#home);
    try {
      state.prepare("INSERT OR REPLACE INTO scans (scan_id, repository_key, document_json, rules_json, queued_at) VALUES (?, ?, ?, ?, ?)")
        .run(report.scanId, report.repository.key, JSON.stringify(report), JSON.stringify(rules), this.#now());
    } finally {
      state.close();
    }
    return true;
  }

  /** Every remote's state, for `rerail remote status` and `rerail status`; no network. */
  status(): RemoteStatus[] {
    const config = readRemoteConfig(this.#home);
    if (config.remotes.length === 0) return [];
    const state = openState(this.#home);
    const rerail = this.#openRerail();
    try {
      const maxSequence = (rerail?.prepare("SELECT max(sequence) AS value FROM events").get() as { value: number | null } | undefined)?.value ?? 0;
      return config.remotes.map(remote => {
        const key = remoteStateKey(remote);
        const row = state.prepare("SELECT * FROM status WHERE remote = ?").get(key) as Record<string, number | string | null> | undefined;
        const cursor = (state.prepare("SELECT value FROM cursors WHERE remote = ? AND stream = 'event'").get(key) as { value: number } | undefined)?.value ?? 0;
        const sentScans = state.prepare("SELECT count(*) AS n FROM scans WHERE scan_id NOT IN (SELECT id FROM sent WHERE remote = ? AND stream = 'scan')").get(key) as { n: number };
        return {
          name: remote.name,
          url: remote.url,
          content: remote.content,
          lastAttemptAt: (row?.last_attempt_at as number | null) ?? null,
          lastSuccessAt: (row?.last_success_at as number | null) ?? null,
          lastError: (row?.last_error as string | null) ?? null,
          consecutiveFailures: (row?.consecutive_failures as number | undefined) ?? 0,
          nextAttemptAt: (row?.next_attempt_at as number | null) ?? null,
          sentBatches: (row?.sent_batches as number | undefined) ?? 0,
          sentRecords: (row?.sent_records as number | undefined) ?? 0,
          pendingEvents: Math.max(0, maxSequence - cursor),
          queuedScans: sentScans.n,
        };
      });
    } finally {
      rerail?.close();
      state.close();
    }
  }

  /**
   * Sends every remote what it lacks, batch after batch, until it is caught up, an attempt fails, or `maxBatches` is
   * reached. `force` ignores the backoff (`rerail remote push`). Never throws for a remote's failure; it is recorded.
   */
  async sync(options: SyncOptions = {}): Promise<SyncResult[]> {
    const config = readRemoteConfig(this.#home);
    if (config.remotes.length === 0) return [];
    const results: SyncResult[] = [];
    for (const remote of config.remotes) {
      if (options.signal?.aborted === true) break;
      results.push(await this.#syncRemote(remote, config.exclusions, options));
    }
    this.#pruneScans(config.remotes.map(remoteStateKey));
    return results;
  }

  /**
   * Drops the outbox's state for a removed remote (by {@link remoteStateKey}). A sync still running for it writes
   * nothing more: every write checks the remote is still configured under that key first.
   */
  forget(key: string): void {
    const state = openState(this.#home);
    try {
      state.transaction(() => {
        for (const table of ["cursors", "sent", "pending_gaps", "status", "locks"]) state.prepare(`DELETE FROM ${table} WHERE remote = ?`).run(key);
      })();
    } finally {
      state.close();
    }
  }

  /** Queued scans every current remote has acknowledged are done with. */
  #pruneScans(remotes: readonly string[]): void {
    const state = openState(this.#home);
    try {
      const placeholders = remotes.map(() => "?").join(", ");
      state.prepare(`DELETE FROM scans WHERE scan_id IN (
        SELECT id FROM sent WHERE stream = 'scan' AND remote IN (${placeholders}) GROUP BY id HAVING count(DISTINCT remote) = ?)`).run(...remotes, remotes.length);
    } finally {
      state.close();
    }
  }

  #openRerail(): Database.Database | undefined {
    const path = join(this.#home, RERAIL_DATABASE_FILE);
    if (!existsSync(path)) return undefined;
    const database = new Database(path, { readonly: true, fileMustExist: true });
    database.pragma("busy_timeout = 5000");
    return database;
  }

  async #syncRemote(remote: RemoteEntry, exclusions: readonly RepositoryExclusion[], options: SyncOptions): Promise<SyncResult> {
    const key = remoteStateKey(remote);
    const token = readRemoteToken(remote.name, this.#home);
    if (token === undefined) return { remote: remote.name, batches: 0, records: 0, skipped: "no-token" };
    const state = openState(this.#home);
    try {
      const configHash = hashOf([remote, exclusions]);
      const row = state.prepare("SELECT next_attempt_at, config_hash FROM status WHERE remote = ?").get(key) as { next_attempt_at: number | null; config_hash: string | null } | undefined;
      // A changed configuration (a new token, a URL) is tried at once.
      const backingOff = row?.next_attempt_at != null && row.next_attempt_at > this.#now() && row.config_hash === configHash;
      if (backingOff && options.force !== true) return { remote: remote.name, batches: 0, records: 0, skipped: "backoff" };
      if (!this.#lock(state, key)) return { remote: remote.name, batches: 0, records: 0, skipped: "locked" };
      try {
        let batches = 0;
        let records = 0;
        const rerail = this.#openRerail();
        try {
          const rules = new RuleDocuments(rerail);
          const budget = options.batchBytes ?? MAX_BATCH_BYTES;
          let maxSendable = Number.POSITIVE_INFINITY;
          let skipped = 0;
          for (let index = 0; index < (options.maxBatches ?? 50); index += 1) {
            if (options.signal?.aborted === true) break;
            const batch = this.#collect(state, rerail, rules, remote, exclusions, budget, maxSendable);
            if (batch.records.length === 0 && batch.oversized.length === 0) break;
            const sendable = batch.records.filter(item => item.line !== "").length;
            if (sendable === 0) {
              // Only opted-out events and records too large to send: nothing goes out, the cursor moves past them.
              this.#acknowledge(state, key, batch.records, batch.oversized, undefined);
              continue;
            }
            const outcome = await this.#post(remote, token, batch.records, options.signal);
            if (!this.#lock(state, key)) return { remote: remote.name, batches, records, skipped: "locked" };
            // Removed (or removed and added again) while the batch was out: nothing more is sent or recorded for it.
            if (!this.#configured(key)) return { remote: remote.name, batches, records, skipped: "removed" };
            if (outcome.kind === "rejected" && sendable > 1) {
              // Find the record the remote refuses before giving anything up: halve the batch and send again.
              maxSendable = Math.max(1, Math.floor(sendable / 2));
              continue;
            }
            if (outcome.kind === "accepted" || outcome.kind === "rejected") {
              this.#acknowledge(state, key, batch.records, batch.oversized, outcome.kind === "rejected" ? outcome.error : undefined);
              batches += 1;
              records += batch.records.length;
              maxSendable = Number.POSITIVE_INFINITY;
              if (outcome.kind === "rejected") {
                skipped += 1;
                // A remote that refuses record after record is broken, not this data: stop, and try again later.
                if (skipped >= MAX_SKIPPED_RECORDS_PER_SYNC) {
                  this.#recordFailure(state, key, configHash, outcome.error, false);
                  return { remote: remote.name, batches, records, error: outcome.error };
                }
              }
              this.#recordSuccess(state, key, configHash, batch.records.length, outcome.kind === "rejected" ? outcome.error : null);
              continue;
            }
            this.#recordFailure(state, key, configHash, outcome.error, outcome.kind === "unauthorized");
            return { remote: remote.name, batches, records, error: outcome.error };
          }
          if (batches === 0) this.#recordSuccess(state, key, configHash, 0, null);
          return { remote: remote.name, batches, records };
        } finally {
          rerail?.close();
        }
      } finally {
        state.prepare("DELETE FROM locks WHERE remote = ? AND holder = ?").run(key, this.#holder);
      }
    } catch (error) {
      return { remote: remote.name, batches: 0, records: 0, error: error instanceof Error ? error.message : String(error) };
    } finally {
      state.close();
    }
  }

  /** One sender per remote across processes (the daemon, `rerail remote push`); a lock left by a dead one expires. */
  #lock(state: Database.Database, remote: string): boolean {
    const now = this.#now();
    const result = state.prepare(`INSERT INTO locks (remote, holder, expires_at) VALUES (?, ?, ?)
      ON CONFLICT (remote) DO UPDATE SET holder = excluded.holder, expires_at = excluded.expires_at
      WHERE locks.holder = excluded.holder OR locks.expires_at < ?`).run(remote, this.#holder, now + LOCK_TTL_MS, now);
    return result.changes > 0;
  }

  #collect(
    state: Database.Database, rerail: Database.Database | undefined, rules: RuleDocuments, remote: RemoteEntry,
    exclusions: readonly RepositoryExclusion[], budget: number, maxSendable: number,
  ): BatchBuilder {
    const key = remoteStateKey(remote);
    const batch = new BatchBuilder(budget - 1024, maxSendable);
    const sent = new Map<string, string>();
    for (const row of state.prepare("SELECT stream, id, hash FROM sent WHERE remote = ?").all(key) as Array<{ stream: string; id: string; hash: string }>)
      sent.set(`${row.stream}\0${row.id}`, row.hash);
    const unsent = (item: OutboxRecord): boolean => item.hash === undefined || sent.get(`${item.stream}\0${item.id}`) !== item.hash;
    const addSnapshot = (item: OutboxRecord): boolean => (unsent(item) ? batch.add(item) : true);
    const excluded = new Exclusions(exclusions, rerail);
    const now = this.#now();

    for (const gap of state.prepare("SELECT id, record_json FROM pending_gaps WHERE remote = ?").all(key) as Array<{ id: string; record_json: string }>)
      if (!batch.add({ line: gap.record_json, stream: "gap", id: gap.id })) return batch;

    if (rerail !== undefined) {
      const repositories = rerail.prepare("SELECT repository_key, display_name, aliases_json FROM repositories").all() as Array<{ repository_key: string; display_name: string; aliases_json: string }>;
      for (const row of repositories) {
        if (excluded.at(row.repository_key, now)) continue;
        const fields = { displayName: row.display_name, aliases: parse(row.aliases_json) };
        if (!addSnapshot(record("repository", row.repository_key, fields, { hash: hashOf(fields) }))) return batch;
      }

      const sessions = rerail.prepare("SELECT session_id, host, cwd, repository_id, started_at, ended_at FROM sessions ORDER BY started_at").all() as Array<{ session_id: string; host: string; cwd: string; repository_id: string | null; started_at: number; ended_at: number | null }>;
      const sessionRepository = new Map<string, string | null>();
      const excludedSessions = new Set<string>();
      for (const row of sessions) {
        const repository = row.repository_id ?? excluded.repositoryOfFolder(row.cwd);
        sessionRepository.set(row.session_id, repository);
        // A session that started while its repository was excluded stays unsent, every event and audit of it too.
        if (excluded.at(repository, row.started_at)) {
          excludedSessions.add(row.session_id);
          continue;
        }
        const fields = { host: row.host, cwd: row.cwd, repositoryKey: row.repository_id, startedAt: row.started_at, endedAt: row.ended_at };
        if (!addSnapshot(record("session", row.session_id, fields, { hash: hashOf(fields) }))) return batch;
      }

      const isExcluded = (sessionId: string, at: number): boolean =>
        excludedSessions.has(sessionId) || excluded.at(sessionRepository.get(sessionId) ?? null, at);
      if (!this.#collectEvents(state, rerail, remote, batch, isExcluded)) return batch;
      if (!this.#collectAudits(rerail, rules, batch, isExcluded, addSnapshot, unsent)) return batch;
    }

    const scans = state.prepare("SELECT scan_id, repository_key, document_json, rules_json, queued_at FROM scans ORDER BY queued_at").all() as Array<{ scan_id: string; repository_key: string; document_json: string; rules_json: string; queued_at: number }>;
    for (const scan of scans) {
      if (excluded.at(scan.repository_key, scan.queued_at)) continue;
      const item: OutboxRecord = { line: JSON.stringify({ type: "scan", id: scan.scan_id, ...(JSON.parse(scan.document_json) as Json) }), stream: "scan", id: scan.scan_id, hash: "queued" };
      if (!unsent(item)) continue;
      for (const rule of JSON.parse(scan.rules_json) as Array<{ ruleId: string; payloadHash: string; checkoutPath: string; document?: RuleDocument }>) {
        if (!this.#addRule(rules, rule, batch, unsent)) return batch;
      }
      if (!batch.add(item)) return batch;
    }
    return batch;
  }

  /** Events after the remote's cursor, a `gap` for every hole in the sequence; false once the batch is full. */
  #collectEvents(state: Database.Database, rerail: Database.Database, remote: RemoteEntry, batch: BatchBuilder, isExcluded: (sessionId: string, at: number) => boolean): boolean {
    const machine = machineId(this.#home);
    let cursor = (state.prepare("SELECT value FROM cursors WHERE remote = ? AND stream = 'event'").get(remoteStateKey(remote)) as { value: number } | undefined)?.value ?? 0;
    for (;;) {
      const rows = rerail.prepare("SELECT sequence, session_id, event_id, kind, observed_at, payload_json FROM events WHERE sequence > ? ORDER BY sequence LIMIT ?")
        .all(cursor, MAX_EVENTS_PER_READ) as Array<{ sequence: number; session_id: string; event_id: string; kind: string; observed_at: number; payload_json: string }>;
      if (rows.length === 0) return true;
      for (const row of rows) {
        if (row.sequence > cursor + 1) {
          const gap = { stream: "event", fromSequence: cursor + 1, toSequence: row.sequence - 1, reason: "deleted-locally" };
          if (!batch.add(record("gap", `${machine}:event:${gap.fromSequence}-${gap.toSequence}`, gap, { sequence: row.sequence - 1 }))) return false;
        }
        cursor = row.sequence;
        const id = `${machine}:${row.sequence}`;
        if (isExcluded(row.session_id, row.observed_at)) {
          // Opted out: the cursor passes it for good with the next record sent, so `include` never sends it later.
          if (!batch.add({ line: "", stream: "skip", id, sequence: row.sequence })) return false;
          continue;
        }
        const content = !remote.content
          ? { content: "omitted" }
          : row.payload_json === V8_ONLY_PAYLOAD ? { payload: null, content: "unrepresentable" } : { payload: parse(row.payload_json) };
        const fields = { sessionId: row.session_id, eventId: row.event_id, kind: row.kind, observedAt: row.observed_at, sequence: row.sequence, ...content };
        if (!batch.add(record("event", id, fields, { sequence: row.sequence }))) return false;
      }
    }
  }

  #collectAudits(
    rerail: Database.Database,
    rules: RuleDocuments,
    batch: BatchBuilder,
    isExcluded: (sessionId: string, at: number) => boolean,
    addSnapshot: (item: OutboxRecord) => boolean,
    unsent: (item: OutboxRecord) => boolean,
  ): boolean {
    const rows = rerail.prepare("SELECT audit_id, session_id, status, started_at, finished_at, length(violations_json) AS size FROM audits WHERE finished_at IS NOT NULL ORDER BY started_at").all() as Array<{ audit_id: string; session_id: string; status: string; started_at: number; finished_at: number; size: number }>;
    let reader: SqliteAuditReader | undefined;
    try {
      for (const row of rows) {
        if (isExcluded(row.session_id, row.started_at)) continue;
        const hash = hashOf([row.status, row.finished_at, row.size]);
        if (!unsent({ line: "", stream: "audit", id: row.audit_id, hash })) continue;
        reader ??= new SqliteAuditReader(join(this.#home, RERAIL_DATABASE_FILE));
        const audit = reader.get(row.audit_id);
        if (audit === undefined) continue;
        for (const provenance of audit.ruleProvenance) {
          const checkoutPath = "checkoutPath" in provenance ? provenance.checkoutPath : audit.workspaceRoot;
          const payloadHash = "payloadHash" in provenance ? provenance.payloadHash : undefined;
          if (payloadHash === undefined) continue;
          if (!this.#addRule(rules, { ruleId: provenance.ruleId, payloadHash, checkoutPath }, batch, unsent)) return false;
        }
        const keys = new Map((rerail.prepare("SELECT ordinal, violation_key, logged FROM audit_violations WHERE audit_id = ?").all(row.audit_id) as Array<{ ordinal: number; violation_key: string; logged: number }>)
          .map(entry => [entry.ordinal, entry]));
        // The audit and its violations go in one batch, or the audit waits for the next.
        const items = [auditRecord(audit, hash), ...violationRecords(audit, keys)];
        const size = items.reduce((total, item) => total + Buffer.byteLength(item.line) + 1, 0);
        if (batch.records.length > 0 && batch.bytes + size > batch.budget) return false;
        for (const item of items) if (!addSnapshot(item)) return false;
      }
      return true;
    } finally {
      reader?.close();
    }
  }

  #addRule(
    rules: RuleDocuments,
    provenance: { ruleId: string; payloadHash: string; checkoutPath: string; document?: RuleDocument },
    batch: BatchBuilder,
    unsent: (item: OutboxRecord) => boolean,
  ): boolean {
    const probe: OutboxRecord = { line: "", stream: "rule", id: provenance.payloadHash, hash: provenance.payloadHash };
    if (!unsent(probe) || batch.ids.has(`rule\0${provenance.payloadHash}`)) return true;
    const document = provenance.document ?? rules.find(provenance);
    if (document === undefined) return true;
    const mode = document.kind === "code" ? (document.rule as { mode?: string }).mode ?? null : null;
    return batch.add(record("rule", provenance.payloadHash, { ruleId: provenance.ruleId, kind: document.kind, mode, document: document.rule }, { hash: provenance.payloadHash }));
  }

  async #post(remote: RemoteEntry, token: string, records: readonly OutboxRecord[], stop?: AbortSignal): Promise<
    { kind: "accepted" } | { kind: "rejected"; error: string } | { kind: "unauthorized"; error: string } | { kind: "retry"; error: string }
  > {
    const header = {
      type: "batch",
      protocol: SINK_PROTOCOL,
      batchId: batchIdOf(records),
      machineId: machineId(this.#home),
      machineLabel: remote.label ?? machineLabel(),
      rerailVersion: rerailVersion(),
      sentAt: this.#now(),
    };
    const lines = [JSON.stringify(header), ...records.filter(item => item.line !== "").map(item => item.line)];
    let response: Response;
    try {
      response = await this.#fetch(new URL(`${remote.url}/v1/batches`), {
        method: "POST",
        headers: { authorization: `Bearer ${token}`, "content-type": "application/x-ndjson" },
        body: `${lines.join("\n")}\n`,
        signal: stop === undefined ? AbortSignal.timeout(REQUEST_TIMEOUT_MS) : AbortSignal.any([AbortSignal.timeout(REQUEST_TIMEOUT_MS), stop]),
      });
    } catch (error) {
      return { kind: "retry", error: error instanceof Error ? error.message : String(error) };
    }
    const body = await response.text().catch(() => "");
    const detail = `${response.status}${body === "" ? "" : ` ${body.slice(0, 200)}`}`;
    if (response.status === 200) return { kind: "accepted" };
    if (response.status === 401 || response.status === 403) return { kind: "unauthorized", error: `unauthorized (${detail})` };
    if (response.status === 400 || response.status === 413) return { kind: "rejected", error: `rejected (${detail})` };
    return { kind: "retry", error: `HTTP ${detail}` };
  }

  /** The remote holds `records` (or refused them for good, `rejected`): cursors and hashes move past them. */
  #acknowledge(state: Database.Database, remote: string, records: readonly OutboxRecord[], oversized: readonly OutboxRecord[], rejected: string | undefined): void {
    state.transaction(() => {
      // Removed, or removed and added again, while this batch was out: its answer belongs to no remote now.
      if (!this.#configured(remote)) return;
      const machine = machineId(this.#home);
      const sequence = [...records, ...oversized].reduce((max, item) => Math.max(max, item.sequence ?? 0), 0);
      if (sequence > 0) state.prepare("INSERT INTO cursors (remote, stream, value) VALUES (?, 'event', ?) ON CONFLICT (remote, stream) DO UPDATE SET value = max(value, excluded.value)").run(remote, sequence);
      const upsert = state.prepare("INSERT INTO sent (remote, stream, id, hash) VALUES (?, ?, ?, ?) ON CONFLICT (remote, stream, id) DO UPDATE SET hash = excluded.hash");
      for (const item of [...records, ...oversized]) if (item.hash !== undefined) upsert.run(remote, item.stream, item.id, item.hash);
      const sentGap = state.prepare("DELETE FROM pending_gaps WHERE remote = ? AND id = ?");
      // Only the gaps this batch carried: a rejected one is reported again inside the new gap's records.
      for (const item of records) if (item.stream === "gap" && item.sequence === undefined) sentGap.run(remote, item.id);
      const addGap = state.prepare("INSERT OR REPLACE INTO pending_gaps (remote, id, record_json) VALUES (?, ?, ?)");
      for (const item of oversized) {
        const id = `${machine}:too-large:${item.stream}:${item.id}`;
        const gap = item.stream === "event"
          ? { stream: "event", fromSequence: item.sequence ?? null, toSequence: item.sequence ?? null, reason: "too-large" }
          : { stream: item.stream, fromSequence: null, toSequence: null, records: [{ stream: item.stream, id: item.id }], reason: "too-large" };
        addGap.run(remote, id, JSON.stringify({ type: "gap", id, ...gap }));
      }
      if (rejected !== undefined) {
        // The remote will never take these: it is told what is missing instead, in the next batch.
        const events = records.filter(item => item.stream === "event").map(item => item.sequence ?? 0);
        const others = records.filter(item => item.stream !== "event" && item.stream !== "gap" && item.stream !== "skip").map(item => ({ stream: item.stream, id: item.id }));
        const gap = {
          stream: "batch",
          fromSequence: events.length === 0 ? null : Math.min(...events),
          toSequence: events.length === 0 ? null : Math.max(...events),
          records: others,
          reason: rejected,
        };
        const id = `${machine}:rejected:${batchIdOf(records)}`;
        addGap.run(remote, id, JSON.stringify({ type: "gap", id, ...gap }));
      }
    })();
  }

  /** Whether a remote is configured under this state key now: the guard on every write of a sync's result. */
  #configured(key: string): boolean {
    return readRemoteConfig(this.#home).remotes.some(remote => remoteStateKey(remote) === key);
  }

  #recordSuccess(state: Database.Database, remote: string, configHash: string, records: number, warning: string | null): void {
    if (!this.#configured(remote)) return;
    const now = this.#now();
    state.prepare(`INSERT INTO status (remote, last_attempt_at, last_success_at, last_error, consecutive_failures, next_attempt_at, config_hash, sent_batches, sent_records)
      VALUES (?, ?, ?, ?, 0, NULL, ?, ?, ?)
      ON CONFLICT (remote) DO UPDATE SET last_attempt_at = excluded.last_attempt_at, last_success_at = excluded.last_success_at,
        last_error = excluded.last_error, consecutive_failures = 0, next_attempt_at = NULL, config_hash = excluded.config_hash,
        sent_batches = sent_batches + excluded.sent_batches, sent_records = sent_records + excluded.sent_records`)
      .run(remote, now, now, warning, configHash, records > 0 ? 1 : 0, records);
  }

  #recordFailure(state: Database.Database, remote: string, configHash: string, error: string, unauthorized: boolean): void {
    if (!this.#configured(remote)) return;
    const now = this.#now();
    const failures = ((state.prepare("SELECT consecutive_failures AS n FROM status WHERE remote = ?").get(remote) as { n: number } | undefined)?.n ?? 0) + 1;
    const delay = unauthorized ? MAX_BACKOFF_MS : backoffMs(failures);
    state.prepare(`INSERT INTO status (remote, last_attempt_at, last_error, consecutive_failures, next_attempt_at, config_hash)
      VALUES (?, ?, ?, ?, ?, ?)
      ON CONFLICT (remote) DO UPDATE SET last_attempt_at = excluded.last_attempt_at, last_error = excluded.last_error,
        consecutive_failures = excluded.consecutive_failures, next_attempt_at = excluded.next_attempt_at, config_hash = excluded.config_hash`)
      .run(remote, now, error, failures, now + delay, configHash);
  }
}

/** Exponential backoff with ±20% jitter, from 2 s to 5 minutes. */
export function backoffMs(failures: number, random: () => number = Math.random): number {
  const base = Math.min(MAX_BACKOFF_MS, MIN_BACKOFF_MS * 2 ** Math.max(0, failures - 1));
  return Math.round(Math.min(MAX_BACKOFF_MS, base * (0.8 + 0.4 * random())));
}

/**
 * The same records make the same batch id, so a batch resent after a lost answer is the batch the remote already
 * stored (C1: the same `batchId` is stored once).
 */
function batchIdOf(records: readonly OutboxRecord[]): string {
  const digest = createHash("sha256");
  for (const item of records) digest.update(`${item.stream}\0${item.id}\0${item.hash ?? ""}\0${item.sequence ?? ""}\n`);
  const hex = digest.digest("hex");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-5${hex.slice(13, 16)}-${((Number.parseInt(hex[16]!, 16) & 0x3) | 0x8).toString(16)}${hex.slice(17, 20)}-${hex.slice(20, 32)}`;
}

let version: string | undefined;
function rerailVersion(): string {
  if (version !== undefined) return version;
  try {
    version = (JSON.parse(readFileSync(join(PACKAGE_ROOT, "package.json"), "utf8")) as { version: string }).version;
  } catch {
    version = "unknown";
  }
  return version;
}

function auditRecord(audit: AuditRecord, hash: string): OutboxRecord {
  return record("audit", audit.auditId, {
    sessionId: audit.sessionId,
    workspaceRoot: audit.workspaceRoot,
    scope: audit.scope,
    mode: audit.mode,
    status: audit.status,
    startedAt: audit.startedAt,
    finishedAt: audit.finishedAt,
    ruleRevisions: audit.ruleRevisions,
    ruleProvenance: audit.ruleProvenance,
    paths: audit.paths,
    failures: audit.failures,
    failureCount: audit.failureCount,
  }, { hash });
}

function violationRecords(audit: AuditRecord, keys: ReadonlyMap<number, { violation_key: string; logged: number }>): OutboxRecord[] {
  const hashes = new Map(audit.ruleProvenance.flatMap(provenance => ("payloadHash" in provenance ? [[provenance.ruleId, provenance.payloadHash] as const] : [])));
  return audit.violations.flatMap((violation, ordinal) => {
    const key = keys.get(ordinal);
    // A repeat of a violation already listed has no row of its own: only the first copy is a finding.
    if (key === undefined) return [];
    const id = `${audit.auditId}:${ordinal}`;
    const fields = {
      auditId: audit.auditId,
      sessionId: audit.sessionId,
      ruleId: violation.ruleId,
      violationKey: key.violation_key,
      logged: key.logged === 1,
      code: violation.code,
      clauseId: violation.clauseId,
      mode: violation.mode,
      message: violation.message,
      path: violation.path ?? null,
      // 1-based, `end` just past the match: the positions of a headless scan's report (`rerail.scan/1`).
      start: violation.span === undefined ? null : { line: violation.span.startLine + 1, column: violation.span.startColumn + 1 },
      end: violation.span === undefined ? null : { line: violation.span.endLine + 1, column: violation.span.endColumn + 1 },
      // The engine's own span, as audits store it: lines and columns 0-based, bytes 0-based, ends exclusive.
      span: violation.span ?? null,
      payloadHash: hashes.get(violation.ruleId) ?? null,
    };
    return [record("violation", id, fields, { hash: hashOf(fields) })];
  });
}

export interface RemoteSinkLoop {
  /** Stops the timer and waits for a sync in progress. */
  stop(): Promise<void>;
}

/**
 * The daemon's sender: every `intervalMs` it syncs each remote whose backoff has passed. It reads the configuration on
 * every tick, so `rerail remote add` and `remove` apply without a restart; with no remote it reads one file and stops.
 * It never throws and never holds the daemon open.
 */
export function startRemoteSink(options: OutboxOptions & { intervalMs?: number; log?: (line: string) => void } = {}): RemoteSinkLoop {
  const outbox = new RemoteOutbox(options);
  const log = options.log ?? (line => process.stderr.write(line));
  let running: Promise<void> | undefined;
  let stopped = false;
  const abort = new AbortController();
  const lastErrors = new Map<string, string>();
  const tick = (): void => {
    if (stopped || running !== undefined) return;
    running = outbox.sync({ signal: abort.signal }).then(results => {
      for (const result of results) {
        // Each new error once, and its recovery, not every retry.
        if (result.error !== undefined && lastErrors.get(result.remote) !== result.error) log(`rerail daemon: remote ${result.remote}: ${result.error}\n`);
        if (result.error === undefined && result.skipped === undefined && lastErrors.has(result.remote)) log(`rerail daemon: remote ${result.remote}: sending again\n`);
        if (result.error !== undefined) lastErrors.set(result.remote, result.error);
        else if (result.skipped === undefined) lastErrors.delete(result.remote);
      }
    }, (error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      if (lastErrors.get("") !== message) log(`rerail daemon: remote sink: ${message}\n`);
      lastErrors.set("", message);
    }).finally(() => { running = undefined; });
  };
  const timer = setInterval(tick, options.intervalMs ?? 10_000);
  timer.unref();
  tick();
  return {
    async stop() {
      stopped = true;
      clearInterval(timer);
      abort.abort();
      await running;
    },
  };
}
