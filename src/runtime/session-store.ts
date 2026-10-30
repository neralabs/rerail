import { randomUUID } from "node:crypto";
import { deserialize, serialize } from "node:v8";
import { and, asc, count, desc, eq, gt, gte, inArray, lt, sql } from "drizzle-orm";
import { z } from "zod";
import type { AdapterSession } from "../engine/adapter-contract.js";
import type { DiscoveryPendingBoundary } from "../engine/code-rule/code-rule-bridge.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { isWorktreePack, WORKTREE_PACK_ID, worktreeRuleCatalog } from "../engine/catalog/worktree-pack.js";
import { LazyReadonlyRerailDatabase, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { decodeCursor, encodeCursor } from "../engine/storage/page-cursor.js";
import { listRepositoryRules, ruleConfigurationSignature } from "../engine/catalog/rule-configuration.js";
import type { RuleCatalog } from "../engine/catalog/repository-model.js";
import { auditViolations, events, sessions, sessionsOfRepository, workLinks, type WorkLinkRow } from "../engine/storage/schema.js";
import { stableJsonText } from "../engine/stable-json.js";
import type { DroppedEvents } from "./session-state.js";

/** The events that wait for the session's code-rule evaluator: a tool call, its result and a turn's end. */
export type CodeRulesPendingBoundary = "call" | "result" | "turn-end";

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
  /** Return the last `limit` events with a sequence strictly less than this value, still in sequence order. */
  before?: number;
  limit?: number;
}

/** What a session's whole transcript says it missed (`SqliteSessionReader.gaps`). */
export interface SessionGaps {
  droppedEvents: number;
  uncheckedBoundaries: number;
  codeRulesPendingBoundaries: number;
}

export interface SessionPageQuery {
  /** Only sessions started at or after this instant. */
  since?: number;
  limit: number;
  /** The previous page's `nextCursor`; the first page without one. */
  cursor?: string;
}

/** Per page row: its history counts and when it last recorded an event. */
export interface SessionPageRow extends StoredSessionSummary {
  /** Violations in the session's history (as `/api/violations?sessionId=` totals it). */
  violationCount: number;
  /** Of those, the ones no audit recorded in Log mode: steered (or blocked). */
  steeredViolations: number;
  /** `observed_at` of the session's newest event; null when it has none. */
  lastEventAt: number | null;
}

export interface SessionPage {
  /** Newest first. */
  sessions: SessionPageRow[];
  /** Every session the query matches, on every page. */
  total: number;
  nextCursor: string | null;
}

/** A session's place in newest-first order: start descending, then id ascending. */
const sessionCursorSchema = z.tuple([z.number().int(), z.string()]);

export interface SessionHud {
  activeRuleCount: number | null;
  steeringCount: number | null;
  /**
   * The labels of the rule overlays in force in the session's checkout while `activeRuleCount` is current: `worktree`
   * while its `.rerail/rules` holds rules that load (`worktree-pack.ts`); empty otherwise.
   */
  overlays: string[];
  /** While `activeRuleCount` is current, how many rules and packs that are on run in Log waiting for a person's approval. */
  awaitingApproval: number | null;
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

  /** Marks the session's history incomplete over a stretch of events its host never delivered. */
  recordDroppedEvents(session: AdapterSession, dropped: DroppedEvents, attribution?: SessionAttribution): void {
    this.append({
      sessionId: session.sessionId,
      eventId: `events-dropped:${dropped.id}`,
      kind: "events-dropped",
      payload: dropped,
    }, session, attribution);
  }

  /** Marks a boundary answered before its shell-write check finished: what that check finds reaches a later boundary. */
  recordDiscoveryPending(session: AdapterSession, boundary: DiscoveryPendingBoundary, attribution?: SessionAttribution): void {
    this.append({
      sessionId: session.sessionId,
      eventId: `source-discovery-pending:${randomUUID()}`,
      kind: "source-discovery-pending",
      payload: { boundary },
    }, session, attribution);
  }

  /** Marks a code-rule boundary answered before the session's evaluator had opened: what it would have checked reaches a later boundary. */
  recordCodeRulesPending(session: AdapterSession, boundary: CodeRulesPendingBoundary, attribution?: SessionAttribution): void {
    this.append({
      sessionId: session.sessionId,
      eventId: `code-rules-pending:${randomUUID()}`,
      kind: "code-rules-pending",
      payload: { boundary },
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
    if (database === undefined) return { activeRuleCount: null, steeringCount: null, overlays: [], awaitingApproval: null };
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
      // Coverage written before repository configuration has no rule signature and is never current. Its first owner
      // is the checkout it was recorded in, whose own rules are in its catalog.
      const owners = coverage?.ownerCheckoutPaths == null ? null : JSON.parse(coverage.ownerCheckoutPaths) as string[];
      const catalog = owners?.[0] === undefined ? loadRuleCatalog() : worktreeRuleCatalog(loadRuleCatalog(), owners[0]);
      const current = coverage?.repositoryId != null && coverage.ruleSignature !== null && owners !== null
        && coverage.ruleSignature === ruleConfigurationSignature(database, catalog, coverage.repositoryId, owners);
      // Per rule, not per hook output (plan D5): the person counts the steer lines they saw, and one edit can steer two
      // rules. A receipt naming no rules still delivered one steer.
      const steeringCount = baseline?.complete === 1
        ? database.select({
          total: sql<number>`coalesce(sum(max(1, coalesce(json_array_length(${events.payloadJson}, '$.ruleIds'), 0))), 0)`,
        }).from(events)
          .where(and(eq(events.sessionId, sessionId), eq(events.kind, "steering-delivered"))).get()!.total
        : null;
      return {
        activeRuleCount: current ? coverage.activeRuleCount : null,
        steeringCount,
        overlays: current && isWorktreePack(catalog.packs.get(WORKTREE_PACK_ID)) ? [WORKTREE_PACK_ID] : [],
        awaitingApproval: current ? awaitingApprovalIn(database, catalog, coverage.repositoryId!, owners) : null,
      };
    });
  }

  /**
   * What the session's whole transcript says it missed, however much of it a reader has loaded: events its host
   * never delivered (`events-dropped` counts), and boundaries that answered before the shell-write check finished or
   * before code rules were running. Indexed by session and kind.
   */
  gaps(sessionId: string): SessionGaps {
    const database = this.#database.open();
    if (database === undefined) return { droppedEvents: 0, uncheckedBoundaries: 0, codeRulesPendingBoundaries: 0 };
    const row = database.all<SessionGaps>(sql`
      SELECT
        coalesce(sum(CASE WHEN ${events.kind} = 'events-dropped' THEN coalesce(json_extract(${events.payloadJson}, '$.count'), 0) END), 0) AS droppedEvents,
        count(*) FILTER (WHERE ${events.kind} = 'source-discovery-pending') AS uncheckedBoundaries,
        count(*) FILTER (WHERE ${events.kind} = 'code-rules-pending') AS codeRulesPendingBoundaries
      FROM ${events}
      WHERE ${events.sessionId} = ${sessionId} AND ${events.kind} IN ('events-dropped', 'source-discovery-pending', 'code-rules-pending')
    `)[0];
    return row ?? { droppedEvents: 0, uncheckedBoundaries: 0, codeRulesPendingBoundaries: 0 };
  }

  events(sessionId: string, query: SessionEventQuery = {}): readonly SessionEventRecord[] {
    const database = this.#database.open();
    if (database === undefined) return [];
    const limit = Math.min(Math.max(1, Math.trunc(query.limit ?? SESSION_EVENT_PAGE_LIMIT)), SESSION_EVENT_PAGE_LIMIT);
    // With `before`, the page is the last `limit` events before it: read backwards, returned in sequence order.
    const backwards = query.before !== undefined;
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
      .where(and(
        eq(events.sessionId, sessionId),
        gt(events.sequence, Math.max(0, Math.trunc(query.after ?? 0))),
        backwards ? lt(events.sequence, Math.trunc(query.before!)) : undefined,
      ))
      .orderBy(backwards ? desc(events.sequence) : asc(events.sequence))
      .limit(limit)
      .all();
    if (backwards) rows.reverse();
    return rows.map(({ payloadJson, ...row }) => ({ ...row, payload: JSON.parse(payloadJson) as unknown }));
  }

  workLinks(sessionId: string): readonly WorkLinkRecord[] {
    const database = this.#database.open();
    return database === undefined ? [] : listWorkLinks(database, sessionId);
  }

  /**
   * One page of sessions, newest first; counts are computed for the page's rows only, so a page costs the same
   * however much history the database holds.
   */
  sessionPage(query: SessionPageQuery, repositoryIds: readonly string[] | undefined = undefined): SessionPage {
    // A malformed cursor is refused whether or not any history exists yet.
    const after = query.cursor === undefined ? undefined : decodeCursor(query.cursor, sessionCursorSchema);
    const database = this.#database.open();
    if (database === undefined) return { sessions: [], total: 0, nextCursor: null };
    const matching = and(
      query.since === undefined ? undefined : gte(sessions.startedAt, query.since),
      repositoryFilter(repositoryIds ?? this.#repositoryIds),
    );
    const continuing = after === undefined
      ? undefined
      : sql`(${sessions.startedAt} < ${after[0]} OR (${sessions.startedAt} = ${after[0]} AND ${sessions.sessionId} > ${after[1]}))`;
    // The page is chosen first and counted after: sorting every session with its counts attached would count them all.
    // The page and its total come from one snapshot, so a commit between them cannot make them disagree.
    const { rows, total } = database.transaction(() => ({
      rows: database.all<SessionPageRow>(sql`
        SELECT
          page.session_id AS sessionId, page.host AS host, page.cwd AS cwd, page.repository_id AS repositoryId,
          page.started_at AS startedAt, page.ended_at AS endedAt,
          (SELECT count(*) FROM ${events} WHERE ${events.sessionId} = page.session_id) AS eventCount,
          (SELECT ${events.observedAt} FROM ${events} WHERE ${events.sessionId} = page.session_id
            ORDER BY ${events.sequence} DESC LIMIT 1) AS lastEventAt,
          (SELECT count(*) FROM ${auditViolations} WHERE ${auditViolations.sessionId} = page.session_id) AS violationCount,
          -- Steered is everything not logged: counted as total minus logged, both index-only (the logged count reads the
          -- partial index audit_violations_key_logged); "logged = 0" would read every violation row of the session.
          (SELECT count(*) FROM ${auditViolations} WHERE ${auditViolations.sessionId} = page.session_id)
            - (SELECT count(*) FROM ${auditViolations}
              WHERE ${auditViolations.sessionId} = page.session_id AND ${auditViolations.logged} = 1) AS steeredViolations
        FROM (
          SELECT * FROM ${sessions}
          WHERE ${and(matching, continuing) ?? sql`1 = 1`}
          ORDER BY ${sessions.startedAt} DESC, ${sessions.sessionId}
          LIMIT ${query.limit + 1}
        ) page
        ORDER BY page.started_at DESC, page.session_id
      `),
      total: database.select({ total: count() }).from(sessions).where(matching).get()?.total ?? 0,
    }));
    const page = rows.slice(0, query.limit);
    const last = page.at(-1);
    return {
      sessions: page,
      total,
      nextCursor: rows.length > query.limit && last !== undefined ? encodeCursor([last.startedAt, last.sessionId]) : null,
    };
  }

  /** How many sessions started at or after `since`. */
  sessionCount(since: number, repositoryIds: readonly string[] | undefined = undefined): number {
    const database = this.#database.open();
    if (database === undefined) return 0;
    return database.select({ total: count() }).from(sessions)
      .where(and(gte(sessions.startedAt, since), repositoryFilter(repositoryIds ?? this.#repositoryIds))).get()?.total ?? 0;
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

/**
 * How many rules and packs (each counted once) the session's checkout runs in Log because their content waits for a
 * person's approval; only rules that are on count.
 */
function awaitingApprovalIn(database: RerailDatabase, catalog: RuleCatalog, repositoryKey: string, owners: readonly string[]): number {
  const [checkoutPath, ...parentCheckoutPaths] = owners;
  const { rules } = listRepositoryRules(database, catalog, repositoryKey, checkoutPath === undefined ? {} : { checkoutPath, parentCheckoutPaths });
  const waiting = new Set(rules.filter(rule => rule.on && rule.approval?.approved === false)
    .map(rule => `${rule.approval!.subject.kind}\0${rule.approval!.subject.id}`));
  return waiting.size;
}

