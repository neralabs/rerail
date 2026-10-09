import { randomUUID } from "node:crypto";
import { deserialize, serialize } from "node:v8";
import { and, asc, count, desc, eq, gt, gte, sql } from "drizzle-orm";
import type { AdapterSession } from "./adapter-contract.js";
import { LazyReadonlyRerailDatabase, openRerailDatabase, type RerailDatabase } from "./database.js";
import { events, sessions, sessionsOfRepository, workLinks, type WorkLinkRow } from "./schema.js";
import { scopedRuleSignature } from "./scoped-rule-catalog.js";
import { stableJsonText } from "./stable-json.js";

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
  /** Repository the session ran in; null when the recorder had no repository identity to attribute it to. */
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

export interface RuleCoverage {
  repositoryId: string;
  activeRuleCount: number | null;
  conversationRuleSignature: string;
  codeRuleSignature: string;
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

/** Limits a shared-database view to the sessions of one repository. */
export interface RepositoryScope {
  repositoryId?: string | undefined;
}

function listSessions(
  database: RerailDatabase,
  { sessionId, startedAtOrAfter, repositoryId }: { sessionId?: string; startedAtOrAfter?: number; repositoryId?: string | undefined } = {},
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
      repositoryId === undefined ? undefined : eq(sessions.repositoryId, repositoryId),
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
 * configured database. Sessions this store opens are attributed to its `repositoryId`.
 */
export class SqliteSessionStore {
  readonly path: string;
  readonly #database: RerailDatabase;
  readonly #repositoryId: string | null;

  constructor(path: string, { repositoryId }: RepositoryScope = {}) {
    this.path = path;
    this.#repositoryId = repositoryId ?? null;
    this.#database = openRerailDatabase(path);
  }

  openSession(session: AdapterSession, startedAt = Date.now()): void {
    this.#database
      .insert(sessions)
      .values({ ...session, repositoryId: this.#repositoryId, startedAt, endedAt: null })
      .onConflictDoUpdate({
        target: sessions.sessionId,
        set: {
          host: session.host,
          cwd: session.cwd,
          endedAt: null,
          ...(this.#repositoryId === null ? {} : { repositoryId: this.#repositoryId }),
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
    lifecycle?: Pick<StoredSessionSummary, "startedAt" | "endedAt">,
  ): void {
    this.append({
      sessionId: session.sessionId,
      eventId: `steering-delivered:${receiptId}`,
      kind: "steering-delivered",
      payload: { ruleIds: [...ruleIds] },
    }, session, lifecycle);
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

  append(event: SessionEvent, session?: AdapterSession, lifecycle?: Pick<StoredSessionSummary, "startedAt" | "endedAt">): string {
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
        repositoryId: this.#repositoryId,
        startedAt: lifecycle?.startedAt ?? Date.now(),
        endedAt: lifecycle?.endedAt ?? null,
      }).onConflictDoNothing().run();
      this.#database.insert(events).values(row).onConflictDoNothing().run();
    }
    return eventId;
  }

  closeSession(sessionId: string, endedAt = Date.now()): void {
    this.#database.update(sessions).set({ endedAt }).where(eq(sessions.sessionId, sessionId)).run();
  }

  sessions(): readonly StoredSessionSummary[] {
    return listSessions(this.#database, { repositoryId: this.#repositoryId ?? undefined });
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
 * With a `repositoryId`, listings and aggregates cover only that repository's sessions; a
 * session addressed by its id is found whichever repository recorded it.
 *
 * The database may not exist yet when a viewer starts before any session has
 * been recorded; reads then report no sessions and the handle is opened on the
 * first read after the writer creates the file.
 */
export class SqliteSessionReader {
  readonly path: string;
  readonly #database: LazyReadonlyRerailDatabase;
  readonly #repositoryId: string | undefined;

  constructor(path: string, { repositoryId }: RepositoryScope = {}) {
    this.path = path;
    this.#repositoryId = repositoryId;
    this.#database = new LazyReadonlyRerailDatabase(path);
  }

  sessions(): readonly StoredSessionSummary[] {
    const database = this.#database.open();
    return database === undefined ? [] : listSessions(database, { repositoryId: this.#repositoryId });
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
        activeRuleCount: sql<number | null>`json_extract(${events.payloadJson}, '$.activeRuleCount')`,
        conversationRuleSignature: sql<string | null>`json_extract(${events.payloadJson}, '$.conversationRuleSignature')`,
        codeRuleSignature: sql<string | null>`json_extract(${events.payloadJson}, '$.codeRuleSignature')`,
      }).from(events).where(and(eq(events.sessionId, sessionId), eq(events.kind, "rule-coverage")))
        .orderBy(desc(events.sequence)).limit(1).get();
      const current = coverage?.repositoryId != null
        && coverage.conversationRuleSignature === scopedRuleSignature(database, coverage.repositoryId, "conversation")
        && coverage.codeRuleSignature === scopedRuleSignature(database, coverage.repositoryId, "code");
      const steeringCount = baseline?.complete === 1
        ? database.select({ total: count() }).from(events)
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
  sessionsSince(since: number): readonly StoredSessionSummary[] {
    const database = this.#database.open();
    return database === undefined ? [] : listSessions(database, { startedAtOrAfter: since, repositoryId: this.#repositoryId });
  }

  /**
   * Steers per rule: steers in range, counted against each rule the delivery
   * named. A delivery that named no rule contributes to nothing.
   */
  steersByRule(since: number): Map<string, number> {
    const database = this.#database.open();
    if (database === undefined) return new Map();
    const repository = this.#repositoryId === undefined
      ? sql``
      : sql`AND event.session_id IN (${sessionsOfRepository(this.#repositoryId)})`;
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
