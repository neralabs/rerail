import { existsSync } from "node:fs";
import type { z } from "zod";
import { loadBundledCatalog } from "../engine/catalog/bundled-rules.js";
import { openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { SqliteAuditReader } from "../engine/audit/audit-store.js";
import { SqlitePresentationPreferenceStore } from "../runtime/presentation-preference-store.js";
import type { BundledCatalog, CheckoutContext, RepositoryStartState } from "../engine/catalog/repository-model.js";
import { createCheckoutResolver } from "../engine/catalog/repository-registry.js";
import { workspaceStartState, startStateReader } from "../runtime/repository-start.js";
import { SqliteAuditControlStore } from "../engine/audit/audit-control.js";
import { SqliteSessionReader, SqliteSessionStore } from "../runtime/session-store.js";
import type { CodingAgentState } from "../engine/adapter-contract.js";
import type { RerailApiStores } from "./operation.js";
import type { presentationDaemonSchema } from "./session-contracts.js";

export interface DatabaseApiStoresOptions {
  readonly health: () => z.infer<typeof presentationDaemonSchema>;
  /** `http://host:port` of the dashboard, named in refusals and links. */
  readonly origin: () => string;
  /** The coding agent check set-up provides (`codingAgentStates`); the API answers 503 without it. */
  readonly codingAgents?: () => Promise<CodingAgentState[]>;
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
  #startStates: ((workspace: string) => RepositoryStartState) | undefined;
  #checkouts: ((workspace: string) => CheckoutContext | undefined) | undefined;
  /** Writes through this instance; `PRAGMA data_version` only counts other connections' commits. */
  #localRevision = 0;

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
    this.#database ??= openRerailDatabase(this.#path);
    return this.#database;
  }

  bundle(): BundledCatalog {
    return loadBundledCatalog();
  }

  checkouts(): (workspace: string) => CheckoutContext | undefined {
    // Nothing is recorded before the database exists, and a passive lookup must not create it.
    if (this.#checkouts === undefined && !existsSync(this.#path)) return () => undefined;
    this.#checkouts ??= createCheckoutResolver(this.database());
    return this.#checkouts;
  }

  dataVersion(): string {
    const sqliteVersion = this.database().$client.pragma("data_version", { simple: true }) as number;
    return `${sqliteVersion}:${this.#localRevision}`;
  }

  markChanged(): void {
    this.#localRevision += 1;
  }

  repositoryState(workspace: string): RepositoryStartState {
    // A daemon with no sessions yet must not create the database to answer that nothing is started.
    if (!existsSync(this.#path)) return workspaceStartState(workspace, this.#path);
    this.#startStates ??= startStateReader(this.database());
    return this.#startStates(workspace);
  }

  close(): void {
    this.reader.close();
    this.#sessionStore?.close();
    this.#auditControls?.close();
    this.audits.close();
    this.#preferences?.close();
    this.#database?.$client.close();
  }
}
