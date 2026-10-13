import { realpathSync } from "node:fs";
import { isAbsolute } from "node:path";
import { createInsertSchema, createSelectSchema } from "drizzle-zod";
import { z } from "zod";
import type { CodeRuleViolation, CodeRuleViolationFix, CodeRuleSourceSpan } from "../engine/code-rule/code-rule.js";
import type { AuditFailure } from "../engine/audit/audit.js";
import type { AuditQuery, RepositoryAuditEntryQuery, RepositoryAuditQuery } from "../engine/audit/audit-store.js";
import { withinCheckout } from "../engine/catalog/checkout-control.js";
import { RERAIL_HOSTS } from "../engine/adapter-contract.js";
import { audits, presentationPreferences, PRESENTATION_HOSTS, events, workLinks } from "../engine/storage/schema.js";
import {
  auditControlsSchema,
  auditSettingsPatchSchema,
  repositoryAuditSchema,
  auditRequestInputSchema,
  auditRequestSchema,
} from "../engine/audit/audit-control-schema.js";
import { RULE_MODES } from "../engine/catalog/rule-mode.js";
import { loadRulePacks } from "../engine/catalog/rule-packs.js";
import { rulePackListSchema } from "./rule-pack-contracts.js";
import { SESSION_EVENT_PAGE_LIMIT } from "../runtime/session-store.js";
import { ApiError, defineOperation, type Operation, type RerailApiStores } from "./operation.js";
import { activityRules, historyRepositoryIds, repositoryOperations } from "./repository-operations.js";
import { inactiveField, inactiveOf } from "./repository-contracts.js";
import { presentationDaemonSchema, presentationResponseSchema, sessionResponseSchema, sessionSummarySchema } from "./session-contracts.js";

/** Query strings arrive as text; MCP arguments arrive typed. Accept both. */
const booleanParameter = z.union([z.boolean(), z.stringbool()]);
const countParameter = z.coerce.number().int().nonnegative();

export const eventRecordSchema = createSelectSchema(events)
  .omit({ payloadJson: true, payloadV8: true })
  .extend({ payload: z.unknown() });

export const workLinkRecordSchema = createSelectSchema(workLinks);
export const workLinkInputSchema = createInsertSchema(workLinks).pick({ kind: true, reference: true, url: true });

const ruleId = z.object({ id: z.string().min(1) });

const codeRuleSourceSpanSchema: z.ZodType<CodeRuleSourceSpan> = z.object({
  startByte: z.number().int(),
  endByte: z.number().int(),
  startLine: z.number().int(),
  startColumn: z.number().int(),
  endLine: z.number().int(),
  endColumn: z.number().int(),
}).strict();

const codeRuleViolationFixSchema: z.ZodType<CodeRuleViolationFix> = z.object({
  span: codeRuleSourceSpanSchema,
  text: z.string().describe("Replacement text for the span. A suggestion only; Rerail never applies it."),
}).strict();

const codeRuleViolationSchema: z.ZodType<CodeRuleViolation> = z.object({
  id: z.string(),
  ruleId: z.string(),
  clauseId: z.string(),
  code: z.string(),
  mode: z.enum(RULE_MODES),
  message: z.string(),
  path: z.string().optional(),
  span: codeRuleSourceSpanSchema.optional(),
  captures: z.record(z.string(), z.array(codeRuleSourceSpanSchema)),
  variables: z.record(z.string(), z.union([z.boolean(), z.number(), z.string()])),
  fix: codeRuleViolationFixSchema.optional(),
}).strict();

const auditFailureSchema: z.ZodType<AuditFailure> = z.object({
  ruleId: z.string().exactOptional(),
  clauseId: z.string(),
  code: z.enum(["budget-exceeded", "missing-capability", "source-incomplete", "source-partial", "adapter-error"]),
  message: z.string(),
  path: z.string().optional(),
  stage: z.enum(["read", "parse", "evaluate", "scan", "scope", "persistence"]).optional(),
}).strict();

export const auditRecordSchema = createSelectSchema(audits).extend({
  paths: z.array(z.string()),
  violations: z.array(codeRuleViolationSchema),
  failures: z.array(auditFailureSchema),
  catalogRevision: z.string().nullable(),
  ruleRevisions: z.array(z.object({ ruleId: z.string(), revision: z.number().int() }).strict()),
});
export const auditControlsResponseSchema = auditControlsSchema.extend({
  repositoryAuditRecord: auditRecordSchema.extend({}).nullable(),
});

export const auditQuerySchema = z.object({
  auditId: z.string().min(1).optional(),
  sessionId: z.string().min(1).optional(),
  status: z.enum(["running", "completed", "failed", "interrupted", "superseded"]).optional(),
  scope: z.enum(["repository", "touched-files", "ai-output"]).optional(),
  mode: z.enum(["steer", "log"]).optional().describe("Audit mode (steer, log), not the mode of the rules it ran."),
  limit: countParameter.min(1).max(200).optional().describe("Maximum records to return; defaults to 50 and is capped at 200."),
  offset: countParameter.optional().describe("Number of matching records to skip; defaults to 0."),
}).strict();
const repositoryAuditQuerySchema = z.object({
  workspaceRoot: z.string().min(1).optional(),
  status: repositoryAuditSchema.shape.status.optional(),
  limit: countParameter.min(1).max(200).optional(),
  offset: countParameter.optional(),
}).strict();
const repositoryAuditSummarySchema = z.object({
  workspaceRoot: z.string(),
  repositoryAudit: repositoryAuditSchema.nullable(),
  audit: z.object({
    auditId: z.string(),
    scope: z.enum(["repository", "touched-files", "ai-output"]),
    status: z.enum(["running", "completed", "failed", "interrupted", "superseded"]),
    startedAt: z.number(),
    finishedAt: z.number().nullable(),
    revision: z.number().int().nonnegative(),
    catalogRevision: z.string().nullable(),
    ruleRevisionCount: z.number().int().nonnegative(),
    violationCount: z.number().int().nonnegative(),
    failureCount: z.number().int().nonnegative(),
  }).strict().nullable(),
}).strict();
const repositoryAuditEntryQuerySchema = z.object({
  kind: z.enum(["violation", "failure"]).optional(),
  mode: z.enum(RULE_MODES).optional().describe("Mode of the rule that produced a violation; failures never match."),
  code: z.string().optional(),
  path: z.string().optional(),
  q: z.string().optional(),
  limit: countParameter.min(1).max(200).optional(),
  offset: countParameter.optional(),
}).strict();
const repositoryAuditEntrySchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("violation"), violation: codeRuleViolationSchema }).strict(),
  z.object({ kind: z.literal("failure"), failure: auditFailureSchema }).strict(),
]);

function auditQuery(input: z.infer<typeof auditQuerySchema>): AuditQuery {
  return {
    ...(input.auditId === undefined ? {} : { auditId: input.auditId }),
    ...(input.sessionId === undefined ? {} : { sessionId: input.sessionId }),
    ...(input.status === undefined ? {} : { status: input.status }),
    ...(input.scope === undefined ? {} : { scope: input.scope }),
    ...(input.mode === undefined ? {} : { mode: input.mode }),
    ...(input.limit === undefined ? {} : { limit: input.limit }),
    ...(input.offset === undefined ? {} : { offset: input.offset }),
  };
}
function repositoryAuditQuery(input: z.infer<typeof repositoryAuditQuerySchema>): RepositoryAuditQuery {
  return {
    ...(input.workspaceRoot === undefined ? {} : { workspaceRoot: input.workspaceRoot }),
    ...(input.status === undefined ? {} : { status: input.status }),
    ...(input.limit === undefined ? {} : { limit: input.limit }),
    ...(input.offset === undefined ? {} : { offset: input.offset }),
  };
}
function repositoryAuditEntryQuery(
  input: z.infer<typeof repositoryAuditEntryQuerySchema> & { auditId: string },
): RepositoryAuditEntryQuery {
  return {
    auditId: input.auditId,
    ...(input.kind === undefined ? {} : { kind: input.kind }),
    ...(input.mode === undefined ? {} : { mode: input.mode }),
    ...(input.code === undefined ? {} : { code: input.code }),
    ...(input.path === undefined ? {} : { path: input.path }),
    ...(input.q === undefined ? {} : { q: input.q }),
    ...(input.limit === undefined ? {} : { limit: input.limit }),
    ...(input.offset === undefined ? {} : { offset: input.offset }),
  };
}
function auditControlsResponse(stores: RerailApiStores, sessionId: string): z.infer<typeof auditControlsResponseSchema> {
  const controls = stores.auditControls().getSettings(sessionId);
  const auditId = controls.repositoryAudit?.auditId;
  return {
    ...controls,
    repositoryAuditRecord: auditId === null || auditId === undefined ? null : stores.audits.get(auditId) ?? null,
  };
}


function requireSession(stores: RerailApiStores, sessionId: string): void {
  if (stores.reader.session(sessionId) === undefined) {
    throw new ApiError(404, "session-not-found", `session ${sessionId} does not exist`);
  }
}

function sessionResponse(stores: RerailApiStores, sessionId: string): z.infer<typeof sessionResponseSchema> | null {
  const session = stores.reader.session(sessionId);
  return session === undefined ? null
    : { session, hud: stores.reader.hud(sessionId), audit: stores.audits.latestSummary(sessionId) ?? null };
}
const sessionId = z.object({ sessionId: z.string().min(1) });

const repositoryFilter = {
  repository: z.string().min(1).optional().describe("Only this repository's history (its key or an alias); machine-wide without it."),
};

/** The session repository ids a `repository` filter matches; undefined (machine-wide) without one. */
function historyIds(stores: RerailApiStores, repository: string | undefined): string[] | undefined {
  return repository === undefined ? undefined : historyRepositoryIds(stores, repository);
}

const sinceParameter = countParameter.describe("Epoch milliseconds; only activity at or after this instant is counted. 0 means since install.");

/**
 * What the catalog knows about one rule's effect over a time range. Conversation rules never write an
 * audit, so they carry no evidence either way: `measured: false` says so rather than
 * reporting a zero that reads as "never triggered". A code rule kept
 * inactive (`inactive`) is not measured either: it is never evaluated.
 */
const ruleActivitySchema = z.object({
  measured: z.boolean(),
  distinctViolations: z.number().int().nonnegative().describe("Distinct (rule, clause, path, span, code) violations; one unfixed problem seen by six audits counts once."),
  sessions: z.number().int().nonnegative(),
  lastSeenAt: z.number().int().nullable(),
  audits: z.number().int().nonnegative().describe("Distinct audits in range that compiled this rule, so a zero violation count over a positive number of audits is evidence rather than silence."),
  loggedViolations: z.number().int().nonnegative().describe("Of distinctViolations, those recorded in Log mode at the time, whatever the rule's mode is now."),
  rerails: z.number().int().nonnegative(),
  ...inactiveField,
}).strict();

const violationRecordSchema = z.object({
  id: z.string(),
  ruleId: z.string(),
  clauseId: z.string(),
  code: z.string(),
  mode: z.enum(RULE_MODES),
  message: z.string(),
  path: z.string().nullable(),
  span: codeRuleSourceSpanSchema.nullable(),
  fix: codeRuleViolationFixSchema.nullable(),
  sessionId: z.string(),
  auditId: z.string(),
  at: z.number().int().describe("Start time of the audit that reported the violation."),
}).strict();

const repositoryPathSchema = z.string().refine(value => value === "" || isAbsolute(value), "Repository must be an absolute path or empty for the database default.");
export const presentationPreferenceSchema = createSelectSchema(presentationPreferences).extend({
  repository: repositoryPathSchema,
}).strict();
const presentationPreferenceKeySchema = presentationPreferenceSchema.pick({ repository: true, host: true });

const codingAgentSchema = z.object({
  id: z.enum(RERAIL_HOSTS),
  name: z.string(),
  found: z.boolean().describe("The coding agent's config folder exists on this computer."),
  connected: z.boolean().describe("Rerail's hooks and commands are in place for it."),
  problem: z.string().nullable().describe("Why the connection could not be checked, such as a config file that is not valid JSON."),
}).strict();

export const operations: readonly Operation[] = [
  defineOperation({
    id: "presentation_preferences_list",
    method: "GET",
    path: "/api/presentation-preferences",
    summary: "List this database's presentation preferences, including database, repository, and harness defaults.",
    output: z.object({ preferences: z.array(presentationPreferenceSchema) }),
    handler: stores => ({ preferences: stores.preferences().list() }),
  }),
  defineOperation({
    id: "presentation_preferences_upsert",
    method: "PUT",
    path: "/api/presentation-preferences",
    summary: "Set compact or silent presentation for a repository and harness. Empty keys select database-local defaults; enforcement is unchanged.",
    body: presentationPreferenceSchema,
    output: z.object({ preference: presentationPreferenceSchema }),
    handler: (stores, input) => ({ preference: stores.preferences().upsert(input) }),
  }),
  defineOperation({
    id: "presentation_preferences_remove",
    method: "DELETE",
    path: "/api/presentation-preferences",
    summary: "Remove a presentation override so the next matching database-local default applies.",
    query: presentationPreferenceKeySchema,
    output: z.object({ removed: z.number().int().nonnegative() }),
    handler: (stores, input) => ({ removed: stores.preferences().remove(input.repository, input.host) }),
  }),
  defineOperation({
    id: "presentation_get",
    method: "GET",
    path: "/api/presentation",
    summary: "Resolve repository+harness, repository, harness, database, then compact presentation; optionally read exact-session status without running an audit.",
    query: z.object({
      repository: z.string().min(1).refine(isAbsolute, "Repository must be an absolute path."),
      host: z.enum(PRESENTATION_HOSTS).exclude([""]),
      sessionId: z.string().min(1).optional().describe("Exact session for passive daemon health and compact status."),
      onDemand: booleanParameter.optional().describe("Read session status even when ambient presentation is silent."),
    }).strict(),
    output: presentationResponseSchema,
    handler: (stores, input) => {
      let session = input.sessionId === undefined ? null : sessionResponse(stores, input.sessionId);
      let repository = input.repository;
      if (session !== null) {
        try {
          // A host may report a directory below its launch root; presence and identity follow the session's canonical cwd.
          const cwd = realpathSync(session.session.cwd);
          if (session.session.host !== input.host || !withinCheckout(cwd, realpathSync(input.repository))) session = null;
          else repository = cwd;
        } catch { session = null; }
      }
      const presence = stores.preferences().effective(repository, input.host);
      const repositoryState = stores.repositoryState(repository);
      const checkout = stores.checkouts()(repository);
      const scope = checkout === undefined ? null : { repositoryKey: checkout.repositoryKey, checkoutPath: checkout.checkoutPath };
      if (input.sessionId === undefined) return { presence, repositoryState, scope };
      const daemon = presentationDaemonSchema.parse(stores.health());
      if (presence === "silent" && !input.onDemand) session = null;
      return { presence, repositoryState, scope, context: { daemon, session } };
    },
  }),
  defineOperation({
    id: "rules_activity",
    method: "GET",
    path: "/api/rules/activity",
    summary: "Report what each rule actually caught since a given time: distinct violations, sessions, last sighting, how many audits evaluated it, and how many steering deliveries it drove. Use it to find rules worth disabling or rules that never fire.",
    query: z.object({ since: sinceParameter, ...repositoryFilter }).strict(),
    output: z.object({ since: z.number().int().nonnegative(), rules: z.record(z.string(), ruleActivitySchema) }),
    handler: (stores, input) => {
      const ids = historyIds(stores, input.repository);
      const activity = stores.audits.ruleActivity(input.since, ids);
      const rerails = stores.reader.steersByRule(input.since, ids);
      const rules: Record<string, z.infer<typeof ruleActivitySchema>> = {};
      for (const { id, kind, rule } of activityRules(stores, input.repository)) {
        // Every rule is reported, so a caller never has to infer absence from a missing key.
        const { inactive } = inactiveOf(kind, rule);
        const totals = kind === "conversation" ? undefined : activity.get(id);
        rules[id] = {
          measured: kind !== "conversation" && inactive === undefined,
          distinctViolations: totals?.distinctViolations ?? 0,
          sessions: totals?.sessions ?? 0,
          lastSeenAt: totals?.lastSeenAt ?? null,
          audits: totals?.audits ?? 0,
          loggedViolations: totals?.loggedViolations ?? 0,
          rerails: kind === "conversation" ? 0 : rerails.get(id) ?? 0,
          ...(inactive === undefined ? {} : { inactive }),
        };
      }
      return { since: input.since, rules };
    },
  }),
  defineOperation({
    id: "rule_packs_list",
    method: "GET",
    path: "/api/rule-packs",
    summary: "List every rule pack that comes with Rerail: what it covers and its rules, each with a title, a group and the mode it starts in. Add a pack to a repository with repository_attachments_set.",
    output: rulePackListSchema,
    handler: () => ({ packs: loadRulePacks().map(({ match: _match, rules, ...pack }) => ({ ...pack, rules: [...rules] })) }),
  }),
  defineOperation({
    id: "coding_agents_list",
    method: "GET",
    path: "/api/coding-agents",
    summary: "List the coding agents Rerail works with: whether each is on this computer and whether Rerail is connected to it. Changes nothing.",
    output: z.object({ agents: z.array(codingAgentSchema) }),
    handler: async (stores) => {
      const agents = stores.codingAgents();
      if (agents === undefined) {
        throw new ApiError(503, "coding-agents-unavailable", "this daemon was started without the coding agent check");
      }
      return { agents: await agents };
    },
  }),
  defineOperation({
    id: "audit_settings_get",
    method: "GET",
    path: "/api/sessions/{sessionId}/audit-settings",
    summary: "Fetch ongoing audit defaults and the first repository audit for one session's canonical workspace.",
    params: sessionId,
    output: auditControlsResponseSchema,
    handler: (stores, input) => {
      requireSession(stores, input.sessionId);
      return auditControlsResponse(stores, input.sessionId);
    },
  }),
  defineOperation({
    id: "audit_settings_update",
    method: "PATCH",
    path: "/api/sessions/{sessionId}/audit-settings",
    summary: "Update ongoing audit defaults for one session's canonical workspace.",
    params: sessionId,
    body: auditSettingsPatchSchema,
    output: auditControlsResponseSchema,
    handler: (stores, input) => {
      requireSession(stores, input.sessionId);
      const { sessionId, ...patch } = input;
      stores.auditControls().updateSettings(sessionId, patch);
      return auditControlsResponse(stores, sessionId);
    },
  }),
  defineOperation({
    id: "audit_requests_create",
    method: "POST",
    path: "/api/sessions/{sessionId}/audit-requests",
    summary: "Queue an explicit audit for one session. Touched-file audits require paths.",
    params: sessionId,
    body: auditRequestInputSchema,
    output: auditRequestSchema,
    handler: (stores, input) => {
      requireSession(stores, input.sessionId);
      const { sessionId, ...request } = input;
      return stores.auditControls().requestAudit(sessionId, request);
    },
  }),
  defineOperation({
    id: "audit_requests_list",
    method: "GET",
    path: "/api/sessions/{sessionId}/audit-requests",
    summary: "List explicit audit requests and the ids of the audits each one produced for one session.",
    params: sessionId,
    output: z.object({ auditRequests: z.array(auditRequestSchema) }),
    handler: (stores, input) => {
      requireSession(stores, input.sessionId);
      return { auditRequests: stores.auditControls().listAuditRequests(input.sessionId) };
    },
  }),
  defineOperation({
    id: "repository_audits_list",
    method: "GET",
    path: "/api/repository-audits",
    summary: "List one full repository audit baseline per historically scanned repository.",
    query: repositoryAuditQuerySchema.extend(repositoryFilter),
    output: z.object({
      repositories: z.array(repositoryAuditSummarySchema),
      total: z.number().int().nonnegative(),
    }),
    handler: (stores, input) => stores.audits.listRepositoryAudits(repositoryAuditQuery(input), historyIds(stores, input.repository)),
  }),
  defineOperation({
    id: "repository_audit_entries_list",
    method: "GET",
    path: "/api/repository-audits/{auditId}/entries",
    summary: "Query paginated violations and failures from one repository audit baseline.",
    params: z.object({ auditId: z.string().min(1) }),
    query: repositoryAuditEntryQuerySchema,
    output: z.object({
      entries: z.array(repositoryAuditEntrySchema),
      total: z.number().int().nonnegative(),
    }),
    handler: (stores, input) => {
      const page = stores.audits.listRepositoryAuditEntries(repositoryAuditEntryQuery(input));
      if (page === undefined) throw new ApiError(404, "repository-audit-not-found", `repository audit ${input.auditId} does not exist`);
      return {
        entries: page.entries.map(entry => entry.kind === "violation"
          ? { kind: "violation" as const, violation: codeRuleViolationSchema.parse(entry.payload) }
          : { kind: "failure" as const, failure: auditFailureSchema.parse(entry.payload) }),
        total: page.total,
      };
    },
  }),
  defineOperation({
    id: "audits_list",
    method: "GET",
    path: "/api/audits",
    summary: "List durable audit history, filtered independently by session, status, scope, or mode.",
    query: auditQuerySchema.extend(repositoryFilter),
    output: z.object({ audits: z.array(auditRecordSchema), total: z.number().int().nonnegative() }),
    handler: (stores, input) => stores.audits.list(auditQuery(input), historyIds(stores, input.repository)),
  }),
  defineOperation({
    id: "violations_list",
    method: "GET",
    path: "/api/violations",
    summary: "List individual violations newest first, optionally for one rule, with the session and audit that reported each. Use it to read what a rule actually flagged rather than how often.",
    query: z.object({
      since: sinceParameter,
      limit: countParameter.min(1).max(200).optional().describe("Maximum violations to return; defaults to 50 and is capped at 200."),
      offset: countParameter.optional().describe("Number of matching violations to skip; defaults to 0."),
      ruleId: z.string().min(1).optional().describe("Restrict to one rule id."),
      ...repositoryFilter,
    }).strict(),
    output: z.object({ total: z.number().int().nonnegative(), violations: z.array(violationRecordSchema) }),
    handler: (stores, input) => {
      const page = stores.audits.listViolationHistory({
        since: input.since,
        ...(input.limit === undefined ? {} : { limit: input.limit }),
        ...(input.offset === undefined ? {} : { offset: input.offset }),
        ...(input.ruleId === undefined ? {} : { ruleId: input.ruleId }),
      }, historyIds(stores, input.repository));
      return {
        total: page.total,
        violations: page.violations.map(({ violation, sessionId, auditId, at }) => {
          const { id, ruleId, clauseId, code, mode, message, path, span, fix } = violation;
          return { id, ruleId, clauseId, code, mode, message, path: path ?? null, span: span ?? null, fix: fix ?? null, sessionId, auditId, at };
        }),
      };
    },
  }),
  defineOperation({
    id: "sessions_list",
    method: "GET",
    path: "/api/sessions",
    summary: "List recorded harness sessions with their event counts, optionally only those started since a given time.",
    query: z.object({
      since: sinceParameter.optional().describe("Epoch milliseconds; only sessions started at or after this instant."),
      ...repositoryFilter,
    }).strict(),
    output: z.object({ sessions: z.array(sessionSummarySchema) }),
    handler: (stores, input) => {
      const ids = historyIds(stores, input.repository);
      return { sessions: [...(input.since === undefined ? stores.reader.sessions(ids) : stores.reader.sessionsSince(input.since, ids))] };
    },
  }),
  defineOperation({
    id: "sessions_get",
    method: "GET",
    path: "/api/sessions/{sessionId}",
    summary: "Fetch one session with rule coverage, steering handoffs and compact audit status.",
    params: sessionId,
    output: sessionResponseSchema,
    handler: (stores, input) => {
      const response = sessionResponse(stores, input.sessionId);
      if (response === null) throw new ApiError(404, "session-not-found", `session ${input.sessionId} does not exist`);
      return response;
    },
  }),
  defineOperation({
    id: "session_events",
    method: "GET",
    path: "/api/sessions/{sessionId}/events",
    summary: `Page through one session's transcript events in sequence order, at most ${SESSION_EVENT_PAGE_LIMIT} per page.`,
    params: sessionId,
    query: z.object({
      after: countParameter.optional().describe("Return events with a sequence strictly greater than this value."),
      limit: countParameter.optional(),
    }),
    output: z.object({
      events: z.array(eventRecordSchema),
      nextAfter: z.number().int().nonnegative(),
      pageLimit: z.literal(SESSION_EVENT_PAGE_LIMIT),
    }),
    handler: (stores, input) => {
      const page = stores.reader.events(input.sessionId, {
        ...(input.after === undefined ? {} : { after: input.after }),
        ...(input.limit === undefined ? {} : { limit: input.limit }),
      });
      const last = page.at(-1);
      return {
        events: [...page],
        nextAfter: last === undefined ? input.after ?? 0 : last.sequence,
        pageLimit: SESSION_EVENT_PAGE_LIMIT as typeof SESSION_EVENT_PAGE_LIMIT,
      };
    },
  }),
  defineOperation({
    id: "work_links_list",
    method: "GET",
    path: "/api/sessions/{sessionId}/work-links",
    summary: "List the external references (branch, pull request, ticket) linked to a session.",
    params: sessionId,
    output: z.object({ workLinks: z.array(workLinkRecordSchema) }),
    handler: (stores, input) => ({ workLinks: [...stores.reader.workLinks(input.sessionId)] }),
  }),
  defineOperation({
    id: "work_links_add",
    method: "PUT",
    path: "/api/sessions/{sessionId}/work-links",
    summary: "Link a session to an external reference. Re-linking the same kind and reference updates its url.",
    params: sessionId,
    body: workLinkInputSchema,
    output: z.object({ workLinks: z.array(workLinkRecordSchema) }),
    handler: (stores, input) => {
      const store = stores.sessionStore();
      store.linkWork({
        sessionId: input.sessionId,
        kind: input.kind,
        reference: input.reference,
        ...(input.url === undefined || input.url === null ? {} : { url: input.url }),
      });
      return { workLinks: [...store.workLinks(input.sessionId)] };
    },
  }),
  ...repositoryOperations,
];

/** The flat argument object an MCP tool takes: path params, query, and body merged. */
export function operationInputShape(operation: Operation): z.ZodRawShape {
  return { ...operation.params?.shape, ...operation.query?.shape, ...operation.body?.shape };
}
