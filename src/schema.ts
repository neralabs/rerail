import { sql, type SQL } from "drizzle-orm";
import { blob, check, index, integer, primaryKey, sqliteTable, text, unique } from "drizzle-orm/sqlite-core";
import { ADAPTER_CAPABILITIES, type RerailHost } from "./adapter-contract.js";
import type { ExecutablePolicyFinding } from "./executable-policy.js";
import {
  POLICY_MODES,
  POLICY_RUN_STATUSES,
  POLICY_SCOPES,
  type PolicyExecutionFailure,
  type PolicyMode,
  type PolicyRunStatus,
  type PolicyScope,
  type RuleEvaluationProvenance,
} from "./policy-execution.js";
import { POLICY_FORMAT, type PolicyCheck, type PolicyScope as PolicyRuleScope } from "./policy-schema.js";
import { RULE_MODES } from "./rule-mode.js";
import type { CosineRule } from "./rule-schema.js";
import type { RuleDocument } from "./rule-store.js";

/**
 * One machine database holds the rule catalog and every repository's execution history.
 * Sessions carry the repository identity that attributes them; rule definitions are
 * resolved for a repository before evaluation.
 *
 * These Drizzle tables are the schema. `schema-sql.ts` is generated from them
 * (`npm run generate:schema`) and applied verbatim when a database is opened.
 * `database.ts` stamps `PRAGMA user_version` with `RERAIL_SCHEMA_VERSION`.
 */
/**
 * The schema version written by this Rerail build. A Rerail refuses a database stamped with a
 * newer version. New tables and indexes need no bump: the DDL is idempotent and older builds
 * ignore what they do not know. A change an older build would misread, or one SQLite cannot
 * apply idempotently, raises it.
 */
export const RERAIL_SCHEMA_VERSION = 1 as const;

export const RULE_KINDS = ["file", "conversation"] as const;
export type RuleKind = (typeof RULE_KINDS)[number];

export const RULE_SCOPES = ["repository", "global"] as const;
export type RuleScope = (typeof RULE_SCOPES)[number];

const inList = (values: readonly string[]): string => values.map(value => `'${value}'`).join(", ");

export const PRESENTATION_HOSTS = ["", ...Object.keys(ADAPTER_CAPABILITIES) as RerailHost[]] as const;
export const PRESENTATION_PRESENCES = ["compact", "silent"] as const;

/** Empty repository/host keys select database-local defaults for that dimension. */
export const presentationPreferences = sqliteTable(
  "presentation_preferences",
  {
    repository: text("repository").notNull(),
    host: text("host", { enum: PRESENTATION_HOSTS }).notNull(),
    presence: text("presence", { enum: PRESENTATION_PRESENCES }).notNull(),
  },
  table => [
    primaryKey({ columns: [table.repository, table.host] }),
    check("presentation_preferences_host", sql`${table.host} IN (${sql.raw(inList(PRESENTATION_HOSTS))})`),
    check("presentation_preferences_presence", sql`${table.presence} IN (${sql.raw(inList(PRESENTATION_PRESENCES))})`),
  ],
);

export const RULE_APPLICABILITIES = ["global", "project"] as const;
export type RuleApplicability = (typeof RULE_APPLICABILITIES)[number];

export const RULE_ORIGINS = ["user", "rerail"] as const;
export type RuleOrigin = (typeof RULE_ORIGINS)[number];

export const rules = sqliteTable(
  "rules",
  {
    id: text("id").primaryKey(),
    kind: text("kind", { enum: RULE_KINDS }).notNull(),
    mode: text("mode", { enum: RULE_MODES }).notNull(),
    revision: integer("revision").notNull(),
    enabled: integer("enabled", { mode: "boolean" }).notNull().default(true),
    scope: text("scope", { enum: RULE_SCOPES }).notNull().default("repository"),
    createdAt: integer("created_at").notNull(),
    updatedAt: integer("updated_at").notNull(),
    /**
     * Digest of the bundled document this row was seeded from, or NULL for a
     * user-created rule. Bootstrap refreshes a row only while its content still
     * matches this digest, so edits survive package updates.
     */
    sourceDigest: text("source_digest"),

    // kind = 'file': one column per top-level field of the executable
    // `rerail.policy/v1` document the kernel compiles. Nested structures are
    // JSON validated by policyRuleSchema at the write boundary.
    format: text("format", { enum: [POLICY_FORMAT] }),
    description: text("description"),
    policyScope: text("scope_json", { mode: "json" }).$type<PolicyRuleScope>(),
    clauses: text("clauses_json", { mode: "json" }).$type<PolicyCheck[]>(),

    // kind = 'conversation': one column per top-level field of the conversation rule schema.
    reference: text("reference"),
    match: text("match_json", { mode: "json" }).$type<CosineRule["match"]>(),
    sample: text("sample_json", { mode: "json" }).$type<CosineRule["sample"]>(),
    action: text("action_json", { mode: "json" }).$type<CosineRule["action"]>(),
    repeat: text("repeat_json", { mode: "json" }).$type<NonNullable<CosineRule["repeat"]>>(),
  },
  table => [
    index("rules_kind_enabled").on(table.kind, table.enabled),
    check("rules_kind", sql`${table.kind} IN (${sql.raw(inList(RULE_KINDS))})`),
    check("rules_mode", sql`${table.mode} IN (${sql.raw(inList(RULE_MODES))})`),
    check("rules_revision", sql`${table.revision} > 0`),
    check("rules_enabled", sql`${table.enabled} IN (0, 1)`),
    check("rules_scope", sql`${table.scope} IN (${sql.raw(inList(RULE_SCOPES))})`),
    check("rules_format", sql`${table.format} IS NULL OR ${table.format} = ${sql.raw(`'${POLICY_FORMAT}'`)}`),
    check("rules_scope_json", sql`${table.policyScope} IS NULL OR json_type(${table.policyScope}) = 'object'`),
    check("rules_clauses_json", sql`${table.clauses} IS NULL OR json_type(${table.clauses}) = 'array'`),
    check("rules_match_json", sql`${table.match} IS NULL OR json_type(${table.match}) = 'object'`),
    check("rules_sample_json", sql`${table.sample} IS NULL OR json_type(${table.sample}) = 'object'`),
    check("rules_action_json", sql`${table.action} IS NULL OR json_type(${table.action}) = 'object'`),
    check("rules_repeat_json", sql`${table.repeat} IS NULL OR json_type(${table.repeat}) = 'object'`),
    check(
      "rules_file_columns",
      sql`${table.kind} <> 'file' OR (
        ${table.format} IS NOT NULL
        AND ${table.clauses} IS NOT NULL
        AND ${table.reference} IS NULL
        AND ${table.match} IS NULL
        AND ${table.sample} IS NULL
        AND ${table.action} IS NULL
        AND ${table.repeat} IS NULL
      )`,
    ),
    check(
      "rules_conversation_columns",
      // A similarity match compares against its reference; a structural match has none.
      sql`${table.kind} <> 'conversation' OR (
        (${table.reference} IS NOT NULL OR json_extract(${table.match}, '$.operator') IS NULL)
        AND ${table.match} IS NOT NULL
        AND ${table.sample} IS NOT NULL
        AND ${table.action} IS NOT NULL
        AND ${table.format} IS NULL
        AND ${table.description} IS NULL
        AND ${table.policyScope} IS NULL
        AND ${table.clauses} IS NULL
      )`,
    ),
  ],
);

/** Machine-local projects group an explicit set of stable repository identities. */
export const ruleProjects = sqliteTable("rule_projects", {
  projectId: text("project_id").primaryKey(),
  name: text("name").notNull(),
  createdAt: integer("created_at").notNull(),
  updatedAt: integer("updated_at").notNull(),
});

/** Repository ids are persisted outside checkout paths so every worktree resolves identically. */
export const ruleRepositories = sqliteTable("rule_repositories", {
  repositoryId: text("repository_id").primaryKey(),
  lastKnownPath: text("last_known_path").notNull(),
  createdAt: integer("created_at").notNull(),
  updatedAt: integer("updated_at").notNull(),
});

export const REPOSITORY_START_STATES = ["started", "stopped"] as const;

/**
 * Whether Rerail is on in a repository's main checkout, and so in every worktree that has no setting of
 * its own (`checkout_starts`). A repository without a row here or there was never set up.
 */
export const repositoryStarts = sqliteTable(
  "repository_starts",
  {
    repositoryId: text("repository_id").primaryKey(),
    state: text("state", { enum: REPOSITORY_START_STATES }).notNull(),
    startedAt: integer("started_at"),
    stoppedAt: integer("stopped_at"),
    updatedAt: integer("updated_at").notNull(),
  },
  table => [check("repository_starts_state", sql`${table.state} IN (${sql.raw(inList(REPOSITORY_START_STATES))})`)],
);

/**
 * A linked worktree's own on/off, which wins over the checkouts that contain it. Without a row a worktree
 * follows the nearest containing checkout of its repository that has one, then the main checkout
 * (`repository_starts`). Set-up and rules stay per repository.
 */
export const checkoutStarts = sqliteTable(
  "checkout_starts",
  {
    repositoryId: text("repository_id").notNull(),
    checkoutPath: text("checkout_path").notNull(),
    state: text("state", { enum: REPOSITORY_START_STATES }).notNull(),
    startedAt: integer("started_at"),
    stoppedAt: integer("stopped_at"),
    updatedAt: integer("updated_at").notNull(),
  },
  table => [
    primaryKey({ columns: [table.repositoryId, table.checkoutPath] }),
    check("checkout_starts_state", sql`${table.state} IN (${sql.raw(inList(REPOSITORY_START_STATES))})`),
  ],
);

/** One logical rule id has one evaluation kind across every owner. */
export const ruleCatalogIdentities = sqliteTable("rule_catalog_identities", {
  ruleId: text("rule_id").primaryKey(),
  kind: text("kind", { enum: RULE_KINDS }).notNull(),
});

export const ruleProjectRepositories = sqliteTable(
  "rule_project_repositories",
  {
    projectId: text("project_id").notNull().references(() => ruleProjects.projectId, { onDelete: "cascade" }),
    repositoryId: text("repository_id").primaryKey().references(() => ruleRepositories.repositoryId, { onDelete: "cascade" }),
    addedAt: integer("added_at").notNull(),
  },
  table => [index("rule_project_repositories_project").on(table.projectId)],
);

/**
 * Authoritative machine catalog. `owner_id` is `global` for global rules and a
 * project id otherwise. The composite key permits a project override to retain
 * the same stable rule id as its inherited global definition.
 */
export const catalogRules = sqliteTable(
  "catalog_rules",
  {
    ownerKind: text("owner_kind", { enum: RULE_APPLICABILITIES }).notNull(),
    ownerId: text("owner_id").notNull(),
    ruleId: text("rule_id").notNull(),
    kind: text("kind", { enum: RULE_KINDS }).notNull(),
    revision: integer("revision").notNull(),
    enabled: integer("enabled", { mode: "boolean" }).notNull().default(true),
    document: text("document_json", { mode: "json" }).$type<RuleDocument>().notNull(),
    payloadHash: text("payload_hash").notNull(),
    sourceDigest: text("source_digest"),
    origin: text("origin", { enum: RULE_ORIGINS }).notNull(),
    createdAt: integer("created_at").notNull(),
    updatedAt: integer("updated_at").notNull(),
  },
  table => [
    primaryKey({ columns: [table.ownerKind, table.ownerId, table.ruleId] }),
    index("catalog_rules_kind_enabled").on(table.kind, table.enabled),
    check("catalog_rules_owner", sql`
      (${table.ownerKind} = 'global' AND ${table.ownerId} = 'global')
      OR (${table.ownerKind} = 'project' AND ${table.ownerId} <> 'global')
    `),
    check("catalog_rules_revision", sql`${table.revision} > 0`),
  ],
);

/**
 * `repository_id` is the stable identity of the checkout the session ran in; per-repository
 * views filter on it. Null when the recorder had no repository identity to attribute it to.
 */
export const sessions = sqliteTable(
  "sessions",
  {
    sessionId: text("session_id").primaryKey(),
    host: text("host").$type<RerailHost>().notNull(),
    cwd: text("cwd").notNull(),
    repositoryId: text("repository_id"),
    startedAt: integer("started_at").notNull(),
    endedAt: integer("ended_at"),
  },
  table => [index("sessions_repository_started").on(table.repositoryId, table.startedAt)],
);

export const events = sqliteTable(
  "events",
  {
    sequence: integer("sequence").primaryKey({ autoIncrement: true }),
    sessionId: text("session_id").notNull().references(() => sessions.sessionId),
    eventId: text("event_id").notNull(),
    kind: text("kind").notNull(),
    observedAt: integer("observed_at").notNull(),
    /** Stable JSON form of the payload; `{ "encoding": "node:v8" }` when it was not JSON-representable. */
    payloadJson: text("payload_json").notNull(),
    payloadV8: blob("payload_v8", { mode: "buffer" }),
  },
  table => [
    unique("events_session_event").on(table.sessionId, table.eventId),
    index("events_session_sequence").on(table.sessionId, table.sequence),
    index("events_session_kind_sequence").on(table.sessionId, table.kind, table.sequence),
  ],
);

/**
 * Loose association from a session to an external reference (git branch,
 * pull request, ticket). Deliberately no foreign key: links may be recorded
 * before or after the session row exists, and by other processes.
 */
export const workLinks = sqliteTable(
  "work_links",
  {
    linkId: integer("link_id").primaryKey({ autoIncrement: true }),
    sessionId: text("session_id").notNull(),
    kind: text("kind").notNull(),
    reference: text("reference").notNull(),
    url: text("url"),
    createdAt: integer("created_at").notNull(),
  },
  table => [
    unique("work_links_session_reference").on(table.sessionId, table.kind, table.reference),
    index("work_links_reference").on(table.kind, table.reference),
  ],
);

/**
 * Durable execution bookkeeping. This deliberately has no session foreign key:
 * evaluation can begin before a harness has recorded its session row, and a
 * reset must not make a live evaluator unable to finish its run.
 */
export const policyRuns = sqliteTable(
  "audits",
  {
    auditId: text("audit_id").primaryKey(),
    sessionId: text("session_id").notNull(),
    workspaceRoot: text("workspace_root").notNull(),
    scope: text("scope", { enum: POLICY_SCOPES }).$type<PolicyScope>().notNull(),
    mode: text("mode", { enum: POLICY_MODES }).$type<PolicyMode>().notNull(),
    revision: integer("revision").notNull(),
    /** Immutable catalog digest the run evaluated against; null when it never reached evaluation. */
    catalogRevision: text("catalog_revision"),
    /** Exact rule revisions compiled for this run; empty when it never reached evaluation. */
    ruleRevisions: text("rule_revisions_json", { mode: "json" }).$type<Array<{ ruleId: string; revision: number }>>().notNull().default([]),
    ruleProvenance: text("rule_provenance_json", { mode: "json" }).$type<RuleEvaluationProvenance[]>().notNull(),
    paths: text("paths_json", { mode: "json" }).$type<string[]>().notNull(),
    startedAt: integer("started_at").notNull(),
    finishedAt: integer("finished_at"),
    status: text("status", { enum: POLICY_RUN_STATUSES }).$type<PolicyRunStatus>().notNull(),
    violations: text("violations_json", { mode: "json" }).$type<ExecutablePolicyFinding[]>().notNull(),
    failures: text("failures_json", { mode: "json" }).$type<PolicyExecutionFailure[]>().notNull(),
  },
  table => [
    index("audits_session_started").on(table.sessionId, table.startedAt, table.auditId),
    index("audits_status").on(table.status),
    index("audits_scope").on(table.scope),
    index("audits_mode").on(table.mode),
    check("audits_scope", sql`${table.scope} IN (${sql.raw(inList(POLICY_SCOPES))})`),
    check("audits_mode", sql`${table.mode} IN (${sql.raw(inList(POLICY_MODES))})`),
    check("audits_status", sql`${table.status} IN (${sql.raw(inList(POLICY_RUN_STATUSES))})`),
    check("audits_revision", sql`${table.revision} >= 0`),
    check("audits_rule_provenance_json", sql`json_type(${table.ruleProvenance}) = 'array'`),
    check("audits_paths_json", sql`json_type(${table.paths}) = 'array'`),
    check("audits_violations_json", sql`json_type(${table.violations}) = 'array'`),
    check("audits_failures_json", sql`json_type(${table.failures}) = 'array'`),
  ],
);

/** Workspace-owned ongoing policy defaults; first-repository diagnostics are fixed log-only scans. */
export const policyRunResultPages = sqliteTable(
  "audit_result_pages",
  {
    auditId: text("audit_id")
      .notNull()
      .references(() => policyRuns.auditId, { onDelete: "cascade" }),
    pageIndex: integer("page_index").notNull(),
    violations: text("violations_json", { mode: "json" }).$type<ExecutablePolicyFinding[]>().notNull(),
    failures: text("failures_json", { mode: "json" }).$type<PolicyExecutionFailure[]>().notNull(),
  },
  table => [
    primaryKey({ columns: [table.auditId, table.pageIndex] }),
    check("audit_result_pages_index", sql`${table.pageIndex} >= 0`),
    check("audit_result_pages_violations_json", sql`json_type(${table.violations}) = 'array'`),
    check("audit_result_pages_failures_json", sql`json_type(${table.failures}) = 'array'`),
  ],
);

export const policySettings = sqliteTable("audit_settings", {
  workspaceRoot: text("workspace_root").primaryKey(),
  settings: text("settings_json", { mode: "json" }).$type<{ scope: PolicyScope; mode: PolicyMode }>().notNull(),
  updatedAt: integer("updated_at").notNull(),
});

/** One repository diagnostic lifecycle per workspace, retried only after a failed or interrupted attempt. */
export const policyRepositoryDiagnostics = sqliteTable("repository_audits", {
  workspaceRoot: text("workspace_root").primaryKey(),
  sessionId: text("session_id").notNull(),
  auditId: text("audit_id"),
  status: text("status", { enum: ["running", "completed", "failed", "interrupted"] as const }).notNull(),
  startedAt: integer("started_at").notNull(),
  finishedAt: integer("finished_at"),
  error: text("error"),
}, table => [
  check("repository_audits_status", sql`${table.status} IN ('running', 'completed', 'failed', 'interrupted')`),
]);

/** User-created scans are durable work items, claimed only by their owning session runtime. */
export const policyScanRequests = sqliteTable(
  "audit_requests",
  {
    id: text("id").primaryKey(),
    sessionId: text("session_id").notNull(),
    workspaceRoot: text("workspace_root").notNull(),
    scope: text("scope", { enum: POLICY_SCOPES }).$type<PolicyScope>().notNull(),
    mode: text("mode", { enum: POLICY_MODES }).$type<PolicyMode>().notNull(),
    paths: text("paths_json", { mode: "json" }).$type<string[]>().notNull(),
    status: text("status", { enum: ["queued", "running", "completed", "failed", "interrupted"] as const }).notNull(),
    createdAt: integer("created_at").notNull(),
    startedAt: integer("started_at"),
    finishedAt: integer("finished_at"),
    auditIds: text("audit_ids_json", { mode: "json" }).$type<string[]>().notNull(),
    error: text("error"),
  },
  table => [
    index("audit_requests_session_created").on(table.sessionId, table.createdAt, table.id),
    index("audit_requests_status").on(table.status),
    check("audit_requests_scope", sql`${table.scope} IN (${sql.raw(inList(POLICY_SCOPES))})`),
    check("audit_requests_mode", sql`${table.mode} IN (${sql.raw(inList(POLICY_MODES))})`),
    check("audit_requests_status_value", sql`${table.status} IN ('queued', 'running', 'completed', 'failed', 'interrupted')`),
    check("audit_requests_paths_json", sql`json_type(${table.paths}) = 'array'`),
    check("audit_requests_audit_ids_json", sql`json_type(${table.auditIds}) = 'array'`),
  ],
);

/** The resident runtime that owns delivery for an active session. */
export const policySessionOwners = sqliteTable("policy_session_owners", {
  sessionId: text("session_id").primaryKey(),
  delivery: text("delivery", { enum: ["background", "next-event"] as const }).notNull(),
  active: integer("active", { mode: "boolean" }).notNull(),
  processId: integer("process_id").notNull(),
  token: text("token").notNull(),
  updatedAt: integer("updated_at").notNull(),
});

/**
 * The ids of `repositoryId`'s sessions, as a subquery for `session_id IN (...)`; `ended` narrows it
 * to sessions that have finished.
 */
export function sessionsOfRepository(repositoryId: string, { ended = false }: { ended?: boolean } = {}): SQL {
  return sql`SELECT ${sessions.sessionId} FROM ${sessions} WHERE ${sessions.repositoryId} = ${repositoryId}${
    ended ? sql` AND ${sessions.endedAt} IS NOT NULL` : sql``
  }`;
}

/**
 * Session history `rerail reset` deletes for every session of the repository, running or not.
 * `audit_result_pages` cascades with `audits`; `sessions` and `policy_session_owners`
 * lose only ended sessions, so running ones keep recording.
 */
export const SESSION_HISTORY_TABLES = [events, workLinks, policyRuns, policyScanRequests, policyRepositoryDiagnostics] as const;

export type RuleRow = typeof rules.$inferSelect;
export type NewRuleRow = typeof rules.$inferInsert;
export type SessionRow = typeof sessions.$inferSelect;
export type EventRow = typeof events.$inferSelect;
export type WorkLinkRow = typeof workLinks.$inferSelect;
export type PolicyRunRow = typeof policyRuns.$inferSelect;
export type PresentationPreference = typeof presentationPreferences.$inferSelect;
export type PresentationPresence = PresentationPreference["presence"];
export type CatalogRuleRow = typeof catalogRules.$inferSelect;
export type NewCatalogRuleRow = typeof catalogRules.$inferInsert;
export type RuleProjectRow = typeof ruleProjects.$inferSelect;
export type RuleRepositoryRow = typeof ruleRepositories.$inferSelect;
export type RepositoryStartRow = typeof repositoryStarts.$inferSelect;
export type RuleProjectRepositoryRow = typeof ruleProjectRepositories.$inferSelect;
export type RuleCatalogIdentityRow = typeof ruleCatalogIdentities.$inferSelect;
