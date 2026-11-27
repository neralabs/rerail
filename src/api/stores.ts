import { existsSync } from "node:fs";
import { join } from "node:path";
import type { z } from "zod";
import { installedPacksPrefix, loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { databaseInstalledAt, openRerailDatabase, openSqliteDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { syncHistoryLog } from "../engine/storage/log-sync.js";
import { onRecordsWritten, openRecordWriter, type RecordWrite, type RecordWriter } from "../engine/storage/record-writes.js";
import { SqliteAuditReader } from "../engine/audit/audit-store.js";
import { SqlitePresentationPreferenceStore } from "../runtime/presentation-preference-store.js";
import type { RuleCatalog, CheckoutContext, RepositoryStartState } from "../engine/catalog/repository-model.js";
import { createCheckoutResolver } from "../engine/catalog/repository-registry.js";
import { workspaceStartState, startStateReader } from "../runtime/repository-start.js";
import { SqliteAuditControlStore } from "../engine/audit/audit-control.js";
import { SqliteSessionReader, SqliteSessionStore } from "../runtime/session-store.js";
import type { CodingAgentState } from "../engine/adapter-contract.js";
import { createMergeAssistService, type MergeAssistRuntime, type MergeAssistService } from "../runtime/merge-assist/service.js";
import type { RerailApiStores } from "./operation.js";
import type { PublishedRulePackReader } from "../engine/catalog/rule-packs.js";
import { databaseChangeFeed, type ChangeFeed, type ChangeFeedOptions } from "./change-feed.js";
import type { presentationDaemonSchema } from "./session-contracts.js";

export interface DatabaseApiStoresOptions {
  readonly health: () => z.infer<typeof presentationDaemonSchema>;
  /** `http://host:port` of the dashboard, named in refusals and links. */
  readonly origin: () => string;
  /** The coding agent check set-up provides (`codingAgentStates`); the API answers 503 without it. */
  readonly codingAgents?: () => Promise<CodingAgentState[]>;
  /** The published rule packs set-up reads from npm (`publishedPackReader`); the API answers 503 without them. */
  readonly publishedRulePacks?: PublishedRulePackReader | undefined;
  /** The daemon's engine service, which starts opening again a session it suspended (`RerailApiStores.resumeSession`). */
  readonly resumeSession?: (sessionId: string) => void;
  /**
   * The Merge Assist service sessions share (the runtime's); without one, a service over this database. Its conflict
   * events, when it tells them, are `conflicts` notices of {@link DatabaseApiStores.changes}.
   */
  readonly mergeAssist?: MergeAssistService & Partial<Pick<MergeAssistRuntime, "onConflictsChanged">>;
  /** How {@link DatabaseApiStores.changes} watches and beats; tests set the beat and drive the notices. */
  readonly changeFeed?: ChangeFeedOptions | undefined;
}

/**
 * Every API store over the one Rerail database, unscoped: an operation names its repository. Writable handles open
 * on first use, so a daemon with no sessions yet does not create the database until asked to.
 */
export class DatabaseApiStores implements RerailApiStores {
  readonly reader: SqliteSessionReader;
  readonly audits: SqliteAuditReader;
  readonly #path: string;
  readonly #options: DatabaseApiStoresOptions;
  #sessionStore: SqliteSessionStore | undefined;
  #preferences: SqlitePresentationPreferenceStore | undefined;
  #auditControls: SqliteAuditControlStore | undefined;
  #database: RerailDatabase | undefined;
  /** {@link dataVersion}'s statement on its own connection (`#versionReader`), once asked for. */
  #dataVersion: { get(): number | undefined } | undefined;
  #versionReader: RerailDatabase | undefined;
  #startStates: ((workspace: string, spelling?: string) => RepositoryStartState) | undefined;
  #checkouts: ((workspace: string) => CheckoutContext | undefined) | undefined;
  #mergeAssist: MergeAssistService | undefined;
  /** The Merge Assist this instance made itself (none was given), which {@link close} stops. */
  #ownMergeAssist: MergeAssistRuntime | undefined;
  /** Writes through this instance; `PRAGMA data_version` only counts other connections' commits. */
  #localRevision = 0;
  /** {@link installedAt} once known; it never changes. */
  #installedAt: number | null = null;
  /**
   * {@link changes} once asked for, and how it stops hearing Merge Assist's conflict events and this process's record
   * writes; all end with the stores.
   */
  #changes: ChangeFeed | undefined;
  #stopConflicts: (() => void) | undefined;
  #stopRecords: (() => void) | undefined;
  /** This instance's use of the database's record writer, for {@link writeWhenFree}: it opens with {@link database} and ends with the stores. */
  #records: RecordWriter | undefined;
  /** Ends what still waits for the write lock in {@link writeWhenFree} as the stores close: nothing more is made. */
  readonly #closing = new AbortController();

  constructor(path: string, options: DatabaseApiStoresOptions) {
    this.#path = path;
    this.#options = options;
    // The read-only views never upgrade, so bring an existing database to the current schema first.
    if (existsSync(path)) openRerailDatabase(path).$client.close();
    this.reader = new SqliteSessionReader(path);
    this.audits = new SqliteAuditReader(path);
  }

  health(): z.infer<typeof presentationDaemonSchema> {
    return this.#options.health();
  }

  codingAgents(): Promise<CodingAgentState[]> | undefined {
    return this.#options.codingAgents?.();
  }

  publishedRulePacks(): PublishedRulePackReader | undefined {
    return this.#options.publishedRulePacks;
  }

  resumeSession(sessionId: string): void {
    this.#options.resumeSession?.(sessionId);
  }

  dashboardOrigin(): string {
    return this.#options.origin();
  }

  sessionStore(): SqliteSessionStore {
    this.#sessionStore ??= new SqliteSessionStore(this.#path);
    return this.#sessionStore;
  }

  preferences(): SqlitePresentationPreferenceStore {
    this.#preferences ??= new SqlitePresentationPreferenceStore(this.#path);
    return this.#preferences;
  }

  auditControls(): SqliteAuditControlStore {
    this.#auditControls ??= new SqliteAuditControlStore(this.#path);
    return this.#auditControls;
  }

  database(): RerailDatabase {
    if (this.#database === undefined) {
      this.#database = openRerailDatabase(this.#path);
      // The record writer's use opens with the handle, as each store opens its own: never first at a write, which can
      // come while another process holds the lock.
      this.#records = openRecordWriter(this.#path);
    }
    return this.#database;
  }

  async writeWhenFree(write: RecordWrite): Promise<boolean> {
    this.database();
    // Opened with the handle above.
    return await this.#records!.writeWhenFree(write, this.#closing.signal);
  }

  ruleCatalog(): RuleCatalog {
    return loadRuleCatalog();
  }

  mergeAssist(): MergeAssistService {
    if (this.#mergeAssist !== undefined) return this.#mergeAssist;
    if (this.#options.mergeAssist !== undefined) {
      this.#mergeAssist = this.#options.mergeAssist;
      return this.#mergeAssist;
    }
    this.#ownMergeAssist = createMergeAssistService({ database: this.database() });
    this.#mergeAssist = this.#ownMergeAssist;
    return this.#mergeAssist;
  }

  checkouts(): (workspace: string) => CheckoutContext | undefined {
    // Nothing is recorded before the database exists, and a passive lookup must not create it.
    if (this.#checkouts === undefined && !existsSync(this.#path)) return () => undefined;
    this.#checkouts ??= createCheckoutResolver(this.database());
    return this.#checkouts;
  }

  dataVersion(): string {
    // Prepared once and stepped at each call, which reads the version anew: preparing it again at every call cost more
    // than the answer, and the change stream asks at each notice and each look.
    this.#dataVersion ??= this.#versionConnection().$client.prepare<[], number>("PRAGMA data_version").pluck();
    const sqliteVersion = this.#dataVersion.get() as number;
    return `${sqliteVersion}:${this.#localRevision}`;
  }

  /**
   * The connection {@link dataVersion} reads on, opened once {@link database} has made and upgraded the file. No write
   * goes through it, and `PRAGMA data_version` moves for every connection's commit but its own reader's, so the commits
   * on {@link database}'s connection (the daemon's Merge Assist writes there) move it as another process's do.
   */
  #versionConnection(): RerailDatabase {
    this.database();
    this.#versionReader ??= openSqliteDatabase(this.#path, { readonly: true });
    return this.#versionReader;
  }

  historyVersion(): string | undefined {
    return existsSync(this.#path) ? this.dataVersion() : undefined;
  }

  /** When Rerail was installed on this machine; null before the database exists, without creating it. */
  installedAt(): number | null {
    if (this.#installedAt !== null || !existsSync(this.#path)) return this.#installedAt;
    this.#installedAt = databaseInstalledAt(this.database());
    return this.#installedAt;
  }

  markChanged(): void {
    this.#localRevision += 1;
    // Its commit's own notice can be heard before this count moves, when the handler awaited after it wrote.
    this.#changes?.notify("data");
  }

  changes(): ChangeFeed {
    if (this.#changes !== undefined) return this.#changes;
    // An install or removal of a pack rewrites the prefix's package files and its node_modules.
    const packs = installedPacksPrefix();
    const feed = databaseChangeFeed(this.#path, () => this.historyVersion(), { files: [packs, join(packs, "node_modules")], ...this.#options.changeFeed });
    // A conflict event of the Merge Assist these stores answer with reaches the stream as it happens.
    this.#stopConflicts = this.#options.mergeAssist?.onConflictsChanged?.(() => feed.notify("conflicts"));
    // So does each commit of this process's record writer (a hook's records, the checks' audits), which its writer tells:
    // on Windows the folder's watch hears none of them (`databaseChangeFeed`).
    this.#stopRecords = onRecordsWritten(this.#path, () => feed.notify("data"));
    this.#changes = feed;
    return feed;
  }

  async syncToDisk(): Promise<void> {
    await syncHistoryLog(this.#path);
  }

  repositoryState(workspace: string, spelling?: string): RepositoryStartState {
    // A daemon with no sessions yet must not create the database to answer that nothing is started.
    if (!existsSync(this.#path)) return workspaceStartState(workspace, this.#path);
    this.#startStates ??= startStateReader(this.database());
    return this.#startStates(workspace, spelling);
  }

  close(): void {
    // Its hook commands get SIGTERM, its passes end at their next Git read, and its late answers record nothing, before
    // the database closes under them: a stop with no wait does all that in its call.
    void this.#ownMergeAssist?.stop(0);
    this.#closing.abort();
    this.#records?.close();
    this.#stopConflicts?.();
    this.#stopRecords?.();
    this.#changes?.close();
    this.reader.close();
    this.#sessionStore?.close();
    this.#auditControls?.close();
    this.audits.close();
    this.#preferences?.close();
    this.#versionReader?.$client.close();
    this.#database?.$client.close();
  }
}
