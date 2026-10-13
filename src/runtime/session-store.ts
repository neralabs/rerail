import { randomUUID } from "node:crypto";
import { deserialize, serialize } from "node:v8";
import { and, asc, count, desc, eq, gt, gte, inArray, sql } from "drizzle-orm";
import type { AdapterSession } from "../engine/adapter-contract.js";
import { loadBundledCatalog } from "../engine/catalog/bundled-rules.js";
import { LazyReadonlyRerailDatabase, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { ruleConfigurationSignature } from "../engine/catalog/rule-configuration.js";
import { events, sessions, sessionsOfRepository, workLinks, type WorkLinkRow } from "../engine/storage/schema.js";
import { stableJsonText } from "../engine/stable-json.js";

export interface SessionEvent {
  sessionId: string;
  kind: string;
  payload: unknown;
  eventId?: string;
  observedAt?: number;
}

export interface StoredSessionSummary {
  sessionId: string;
  host: AdapterSession["host"];
  cwd: string;
  /** Repository key the session ran in; null when the recorder had no repository to attribute it to. */
  repositoryId: string | null;
  startedAt: number;
  endedAt: number | null;
  eventCount: number;
}

export interface SessionEventRecord {
  sequence: number;
  sessionId: string;
  eventId: string;
  kind: string;
  observedAt: number;
  /** The stable JSON form of the payload; `{ "encoding": "node:v8" }` when it was not JSON-representable. */
  payload: unknown;
}

export interface SessionEventQuery {
  /** Return events with a sequence strictly greater than this value. */
  after?: number;
  limit?: number;
}

export interface SessionHud {
  activeRuleCount: number | null;
  steeringCount: number | null;
}

/** What a session's active-rule count was computed from, so the footer can tell whether it is still current. */
export interface RuleCoverage {
  repositoryId: string;
  /** The starting checkout and its parent checkouts, whose settings apply. */
  ownerCheckoutPaths: readonly string[];
  activeRuleCount: number | null;
  /** The repository's rule configuration signature the count was loaded at. */
  ruleSignature: string;
}

export interface WorkLink {
  sessionId: string;
  /** Reference namespace, e.g. `git-branch`, `github-pr`, `linear-issue`. Free-form. */
  kind: string;
  /** Branch name, pull request number, ticket identifier. */
  reference: string;
  url?: string;
}

export type WorkLinkRecord = WorkLinkRow;

export const SESSION_EVENT_PAGE_LIMIT = 500;

function isForeignKeyFailure(error: unknown): boolean {
  const cause = error instanceof Error && "cause" in error ? error.cause : error;
  return typeof cause === "object" && cause !== null && "code" in cause && cause.code === "SQLITE_CONSTRAINT_FOREIGNKEY";
}

/** Limits a shared-database view to the sessions of one repository (its key and, for older rows, clone ids). */
export interface RepositoryScope {
  repositoryId?: string | readonly string[] | undefined;
}

/** Who a session row belongs to when a write has to (re)create it. */
export interface SessionAttribution {
  startedAt?: number;
  endedAt?: number | null;
  repositoryKey?: string | null;
}

function repositoryFilter(repositoryIds: string | readonly string[] | undefined) {
  if (repositoryIds === undefined) return undefined;
  const ids = typeof repositoryIds === "string" ? [repositoryIds] : [...repositoryIds];
  return ids.length === 0 ? sql`0` : inArray(sessions.repositoryId, ids);
}

function listSessions(
  database: RerailDatabase,
  { sessionId, startedAtOrAfter, repositoryIds }: {
    sessionId?: string;
    startedAtOrAfter?: number;
    repositoryIds?: string | readonly string[] | undefined;
  } = {},
): StoredSessionSummary[] {
  return database
    .select({
      sessionId: sessions.sessionId,
      host: sessions.host,
      cwd: sessions.cwd,
      repositoryId: sessions.repositoryId,
      startedAt: sessions.startedAt,
      endedAt: sessions.endedAt,
      eventCount: count(events.sequence),
    })
    .from(sessions)
    .leftJoin(events, eq(events.sessionId, sessions.sessionId))
    .where(and(
      sessionId === undefined ? undefined : eq(sessions.sessionId, sessionId),
      startedAtOrAfter === undefined ? undefined : gte(sessions.startedAt, startedAtOrAfter),
      repositoryFilter(repositoryIds),
    ))
    .groupBy(sessions.sessionId)
    .orderBy(asc(sessions.startedAt), asc(sessions.sessionId))
    .all();
}

function listWorkLinks(database: RerailDatabase, sessionId: string): WorkLinkRecord[] {
  return database.select().from(workLinks).where(eq(workLinks.sessionId, sessionId)).orderBy(asc(workLinks.linkId)).all();
}


export function decodeEventPayload(payload: Uint8Array): unknown {
  return deserialize(payload);
}

/**
 * Append-only, process-safe session and event persistence. Raw event payloads remain local to the
 * configured database. Sessions are attributed to the repository key given when they open, else to the
 * store's own `repositoryId`.
 */
export class SqliteSessionStore {
  readonly path: string;
  readonly #database: RerailDatabase;
  readonly #repositoryId: string | null;
  /** Each session's repository as it opened here, to restore its row after a reset. */
  readonly #sessionRepositories = new Map<string, string | null>();

  constructor(path: string, { repositoryId }: { repositoryId?: string | undefined } = {}) {
    this.path = path;
    this.#repositoryId = repositoryId ?? null;
    this.#database = openRerailDatabase(path);
  }

  openSession(session: AdapterSession, startedAt = Date.now(), repositoryKey: string | null = this.#repositoryId): void {
    this.#sessionRepositories.set(session.sessionId, repositoryKey);
    this.#database
      .insert(sessions)
      .values({ ...session, repositoryId: repositoryKey, startedAt, endedAt: null })
      .onConflictDoUpdate({
        target: sessions.sessionId,
        set: {
          host: session.host,
          cwd: session.cwd,
          endedAt: null,
          ...(repositoryKey === null ? {} : { repositoryId: repositoryKey }),
        },
      })
      .run();
  }

  /** A stable identity makes reopen idempotent; missing history can never become complete later. */
  startHudTracking(session: AdapterSession, complete: boolean): void {
    this.append({
      sessionId: session.sessionId,
      eventId: "hud-tracking-start",
      kind: "hud-tracking-start",
      payload: { complete },
    }, session);
  }

  /** `ruleIds` credits the steer to the rules whose violations it carried. */
  recordSteer(
    session: AdapterSession,
    receiptId: string,
    ruleIds: readonly string[] = [],
    attribution?: SessionAttribution,
  ): void {
    this.append({
      sessionId: session.sessionId,
      eventId: `steering-delivered:${receiptId}`,
      kind: "steering-delivered",
      payload: { ruleIds: [...ruleIds] },
    }, session, attribution);
  }


  recordRuleCoverage(session: AdapterSession, coverage: RuleCoverage): void {
    const latest = this.#database.select({ payloadJson: events.payloadJson }).from(events)
      .where(and(eq(events.sessionId, session.sessionId), eq(events.kind, "rule-coverage")))
      .orderBy(desc(events.sequence)).limit(1).get();
    // Streaming ingestion checks coverage often; persist transitions, not identical copies.
    // Looking in SQLite rather than an in-memory cache also recovers after a reset.
    if (latest?.payloadJson === stableJsonText(coverage)) return;
    this.append({ sessionId: session.sessionId, kind: "rule-coverage", payload: coverage }, session);
  }

  append(event: SessionEvent, session?: AdapterSession, attribution?: SessionAttribution): string {
    const eventId = event.eventId ?? randomUUID();
    const row = {
      sessionId: event.sessionId,
      eventId,
      kind: event.kind,
      observedAt: event.observedAt ?? Date.now(),
      payloadJson: stableJsonText(event.payload) ?? JSON.stringify({ encoding: "node:v8" }),
      payloadV8: serialize(event.payload),
    };
    try {
      this.#database.insert(events).values(row).onConflictDoNothing().run();
    } catch (error) {
      // A `rerail reset` rebuilt the tables underneath a running session: its row is gone, so put
      // it back and keep going. The events before the reset are gone by design; these are not.
      if (session === undefined || !isForeignKeyFailure(error)) throw error;
      this.#database.insert(sessions).values({
        ...session,
        repositoryId: attribution?.repositoryKey ?? this.#sessionRepositories.get(session.sessionId) ?? this.#repositoryId,
        startedAt: attribution?.startedAt ?? Date.now(),
        endedAt: attribution?.endedAt ?? null,
      }).onConflictDoNothing().run();
      this.#database.insert(events).values(row).onConflictDoNothing().run();
    }
    return eventId;
  }

  closeSession(sessionId: string, endedAt = Date.now()): void {
    this.#database.update(sessions).set({ endedAt }).where(eq(sessions.sessionId, sessionId)).run();
  }

  sessions(): readonly StoredSessionSummary[] {
    return listSessions(this.#database, { repositoryIds: this.#repositoryId ?? undefined });
  }

  session(sessionId: string): StoredSessionSummary | undefined {
    return listSessions(this.#database, { sessionId })[0];
  }

  linkWork(link: WorkLink, createdAt = Date.now()): void {
    this.#database
      .insert(workLinks)
      .values({ ...link, url: link.url ?? null, createdAt })
      .onConflictDoUpdate({
        target: [workLinks.sessionId, workLinks.kind, workLinks.reference],
        set: { url: link.url ?? null },
      })
      .run();
  }

  workLinks(sessionId: string): readonly WorkLinkRecord[] {
    return listWorkLinks(this.#database, sessionId);
  }

  close(): void {
    this.#database.$client.close();
  }
}

/**
 * Read-only view over a database written by {@link SqliteSessionStore}.
 *
 * With a `repositoryId` (a key, or a key and the clone ids of its checkouts), listings and aggregates cover
 * only that repository's sessions; a call that names `repositoryIds` uses those instead. A session addressed
 * by its id is found whichever repository recorded it.
 *
 * The database may not exist yet when a viewer starts before any session has
 * been recorded; reads then report no sessions and the handle is opened on the
 * first read after the writer creates the file.
 */
export class SqliteSessionReader {
  readonly path: string;
  readonly #database: LazyReadonlyRerailDatabase;
  readonly #repositoryIds: string | readonly string[] | undefined;

  constructor(path: string, { repositoryId }: RepositoryScope = {}) {
    this.path = path;
    this.#repositoryIds = repositoryId;
    this.#database = new LazyReadonlyRerailDatabase(path);
  }

  sessions(repositoryIds: readonly string[] | undefined = undefined): readonly StoredSessionSummary[] {
    const database = this.#database.open();
    return database === undefined ? [] : listSessions(database, { repositoryIds: repositoryIds ?? this.#repositoryIds });
  }

  session(sessionId: string): StoredSessionSummary | undefined {
    const database = this.#database.open();
    return database === undefined ? undefined : listSessions(database, { sessionId })[0];
  }

  hud(sessionId: string): SessionHud {
    const database = this.#database.open();
    if (database === undefined) return { activeRuleCount: null, steeringCount: null };
    // Read coverage, lifetime totals, and the catalog signature from one snapshot.
    return database.transaction(() => {
      const baseline = database.select({
        complete: sql<number | null>`json_extract(${events.payloadJson}, '$.complete')`,
      }).from(events).where(and(
        eq(events.sessionId, sessionId), eq(events.eventId, "hud-tracking-start"), eq(events.kind, "hud-tracking-start"),
      )).get();
      const coverage = database.select({
        repositoryId: sql<string | null>`json_extract(${events.payloadJson}, '$.repositoryId')`,
        ownerCheckoutPaths: sql<string | null>`json_extract(${events.payloadJson}, '$.ownerCheckoutPaths')`,
        activeRuleCount: sql<number | null>`json_extract(${events.payloadJson}, '$.activeRuleCount')`,
        ruleSignature: sql<string | null>`json_extract(${events.payloadJson}, '$.ruleSignature')`,
      }).from(events).where(and(eq(events.sessionId, sessionId), eq(events.kind, "rule-coverage")))
        .orderBy(desc(events.sequence)).limit(1).get();
      // Coverage written before repository configuration has no rule signature and is never current.
      const current = coverage?.repositoryId != null && coverage.ruleSignature !== null && coverage.ownerCheckoutPaths !== null
        && coverage.ruleSignature === ruleConfigurationSignature(
          database, loadBundledCatalog(), coverage.repositoryId, JSON.parse(coverage.ownerCheckoutPaths) as string[],
        );
      // Per rule, not per hook output (plan D5): the person counts the steer lines they saw, and one edit can steer two
      // rules. A receipt naming no rules still delivered one steer.
      const steeringCount = baseline?.complete === 1
        ? database.select({
          total: sql<number>`coalesce(sum(max(1, coalesce(json_array_length(${events.payloadJson}, '$.ruleIds'), 0))), 0)`,
        }).from(events)
          .where(and(eq(events.sessionId, sessionId), eq(events.kind, "steering-delivered"))).get()!.total
        : null;
      return { activeRuleCount: current ? coverage.activeRuleCount : null, steeringCount };
    });
  }

  events(sessionId: string, query: SessionEventQuery = {}): readonly SessionEventRecord[] {
    const database = this.#database.open();
    if (database === undefined) return [];
    const limit = Math.min(Math.max(1, Math.trunc(query.limit ?? SESSION_EVENT_PAGE_LIMIT)), SESSION_EVENT_PAGE_LIMIT);
    const rows = database
      .select({
        sequence: events.sequence,
        sessionId: events.sessionId,
        eventId: events.eventId,
        kind: events.kind,
        observedAt: events.observedAt,
        payloadJson: events.payloadJson,
      })
      .from(events)
      .where(and(eq(events.sessionId, sessionId), gt(events.sequence, Math.max(0, Math.trunc(query.after ?? 0)))))
      .orderBy(asc(events.sequence))
      .limit(limit)
      .all();
    return rows.map(({ payloadJson, ...row }) => ({ ...row, payload: JSON.parse(payloadJson) as unknown }));
  }

  workLinks(sessionId: string): readonly WorkLinkRecord[] {
    const database = this.#database.open();
    return database === undefined ? [] : listWorkLinks(database, sessionId);
  }

  /** Sessions that started at or after `since`, for a page showing one time range. */
  sessionsSince(since: number, repositoryIds: readonly string[] | undefined = undefined): readonly StoredSessionSummary[] {
    const database = this.#database.open();
    return database === undefined
      ? []
      : listSessions(database, { startedAtOrAfter: since, repositoryIds: repositoryIds ?? this.#repositoryIds });
  }

  /**
   * Steers per rule: steers in range, counted against each rule the delivery
   * named. A delivery that named no rule contributes to nothing.
   */
  steersByRule(since: number, repositoryIds: readonly string[] | undefined = undefined): Map<string, number> {
    const database = this.#database.open();
    if (database === undefined) return new Map();
    const scope = repositoryIds ?? this.#repositoryIds;
    const repository = scope === undefined
      ? sql``
      : sql`AND event.session_id IN (${sessionsOfRepository(scope)})`;
    const rows = database.all<{ ruleId: string; deliveries: number }>(sql`
      SELECT rule.value AS ruleId, count(DISTINCT event.sequence) AS deliveries
      FROM ${events} event, json_each(json_extract(event.payload_json, '$.ruleIds')) rule
      WHERE event.kind = 'steering-delivered' AND event.observed_at >= ${since} ${repository}
      GROUP BY rule.value
    `);
    return new Map(rows.map(row => [row.ruleId, row.deliveries]));
  }

  close(): void {
    this.#database.close();
  }
}
