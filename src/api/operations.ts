import { isAbsolute } from "node:path";
import { createInsertSchema, createSelectSchema } from "drizzle-zod";
import { z } from "zod";
import { SECRET_FILE_FINDING_MESSAGE, type CodeRuleViolation, type CodeRuleViolationFix, type CodeRuleSourceSpan } from "../engine/code-rule/code-rule.js";
import { AUDIT_BLIND_REASONS, AUDIT_FAILURE_CODES, AUDIT_MODES, AUDIT_ORIGINS, AUDIT_SCOPES, AUDIT_STATUSES, type AuditFailure } from "../engine/audit/audit.js";
import type { AuditQuery, RepositoryAuditEntryQuery, RepositoryAuditQuery } from "../engine/audit/audit-store.js";
import { checkoutRealpath, withinCheckout } from "../engine/catalog/checkout-control.js";
import { RERAIL_HOSTS } from "../engine/adapter-contract.js";
import { internalTesting } from "../engine/internal-testing.js";
import { agentTranscriptPayload } from "../engine/transcript-boundary.js";
import { CHECKOUT_PATH_RESOLUTIONS, checkoutPathBoundary, sensitiveByName, type CheckoutPathBoundary } from "../engine/path-boundary.js";
import { audits, presentationPreferences, PRESENTATION_HOSTS, events, workLinks } from "../engine/storage/schema.js";
import {
  auditControlsSchema,
  auditSettingsPatchSchema,
  repositoryAuditSchema,
  auditRequestInputSchema,
  auditRequestSchema,
} from "../engine/audit/audit-control-schema.js";
import { RULE_MODES } from "../engine/catalog/rule-mode.js";
import { approvalKey, packSubject, readApprovals } from "../engine/catalog/rule-approval.js";
import { rulePackListSchema } from "./rule-pack-contracts.js";
import { SESSION_EVENT_PAGE_LIMIT, type DeclinedViolations, type DeclineNote, type StoredSessionSummary } from "../runtime/session-store.js";
import type { ApiCaller } from "../engine/catalog/repository-model.js";
import { ApiError, defineOperation, experimentalSummary, type Operation, type RerailApiStores } from "./operation.js";
import { mergeAssistOperations } from "./merge-assist-operations.js";
import { activityRules, callerRepositoryKey, checkoutAt, checkoutRuleCatalog, historyScope, repositoryOperations, within } from "./repository-operations.js";
import { inactiveField, inactiveOf, targetShape } from "./repository-contracts.js";
import { presentationDaemonSchema, presentationResponseSchema, sessionResponseSchema, sessionSummarySchema } from "./session-contracts.js";

/** How many of a session's `source-discovery-gap` places `sessions_get` lists: what the dashboard shows of them. */
const DISCOVERY_GAP_PLACES = 20;

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

/** A boundary for paths under `root` when it is an absolute folder whose real path lies inside `checkout`; else none. */
function boundaryInside(checkout: string, root: string): CheckoutPathBoundary | undefined {
  if (!isAbsolute(root)) return undefined;
  try {
    const real = checkoutRealpath(root);
    return within(checkout, real) === undefined ? undefined : checkoutPathBoundary(real);
  } catch {
    // The folder is gone since its audit ran.
    return undefined;
  }
}

/**
 * Whether a finding's path is secret-like for a coding agent at work in `checkout`: by its name, or by where it leads
 * from `root`, the folder its audit ran in. The runtime keeps the path relative to that folder and its steer judged it
 * there, so from a session in `web`, `conf/config.py` with `web/conf` a link to `../.aws` is one. A folder outside
 * `checkout` (a worktree beside it, one a link leads out to, one gone or not recorded) is not read: its findings are
 * judged by name. One judge serves one answer: it reads each folder's real path once and resolves at most
 * CHECKOUT_PATH_RESOLUTIONS distinct paths across all its folders, past which it judges by name.
 */
function recordedPathJudge(checkout: string): (root: string | undefined, path: string) => boolean {
  const boundaries = new Map<string, CheckoutPathBoundary | undefined>();
  const resolved = new Set<string>();
  const boundaryOf = (root: string | undefined): CheckoutPathBoundary | undefined => {
    if (root === undefined) return undefined;
    if (!boundaries.has(root)) boundaries.set(root, boundaryInside(checkout, root));
    return boundaries.get(root);
  };
  return (root, path) => {
    if (sensitiveByName(path)) return true;
    const boundary = boundaryOf(root);
    const key = `${root}\0${path}`;
    if (boundary === undefined || (!resolved.has(key) && resolved.size >= CHECKOUT_PATH_RESOLUTIONS)) return false;
    resolved.add(key);
    return boundary.sensitive(path);
  };
}

/**
 * Violations as `caller` may see them, each with the folder its audit ran in. A coding agent sees one in a secret-like
 * file ({@link recordedPathJudge}) by rule and place only, as the in-session steer says it: the message may carry the
 * file's text through the rule's metavariables, `variables` are that text, and the fix is its replacement; a person
 * reads them in the dashboard. Make one view per answer.
 */
function violationView(caller: ApiCaller): (violation: CodeRuleViolation, root: string | undefined) => CodeRuleViolation {
  if (caller.kind === "person") return violation => violation;
  // Defensive: every list calls historyScope first, which refuses an agent with no checkout.
  const sensitive = caller.checkout === undefined ? (_root: string | undefined, path: string) => sensitiveByName(path) : recordedPathJudge(caller.checkout);
  return (violation, root) => {
    if (violation.path === undefined || !sensitive(root, violation.path)) return violation;
    const { fix: _fix, ...rest } = violation;
    return { ...rest, message: SECRET_FILE_FINDING_MESSAGE, variables: {} };
  };
}

/** Rule `id`'s declined violations as `rules_activity` reports them; zeros for a rule with none. */
function declinedOf(declined: ReadonlyMap<string, DeclinedViolations>, id: string) {
  const counts = declined.get(id);
  return { declinedViolations: counts?.declined ?? 0, declinedLoggedViolations: counts?.alsoLogged ?? 0 };
}

/** A steer as `violations_list` names it: the audit that steered and the rule it steered on. */
const steerKey = (auditId: string, ruleId: string) => `${auditId}\u0000${ruleId}`;

/** The reasons of decline notes, oldest first, by the steer each declined. */
function declineReasonsBySteer(notes: ReadonlyArray<DeclineNote & { auditId: string }>): Map<string, string[]> {
  const reasons = new Map<string, string[]>();
  for (const note of notes) {
    const key = steerKey(note.auditId, note.ruleId);
    reasons.set(key, [...reasons.get(key) ?? [], note.reason]);
  }
  return reasons;
}

const auditFailureSchema: z.ZodType<AuditFailure> = z.object({
  ruleId: z.string().exactOptional(),
  clauseId: z.string(),
  code: z.enum(AUDIT_FAILURE_CODES),
  message: z.string(),
  path: z.string().optional(),
  stage: z.enum(["read", "parse", "evaluate", "scan", "scope", "persistence"]).optional(),
  blind: z.enum(AUDIT_BLIND_REASONS).optional(),
}).strict();

// `ruleProvenance` is the audit's provenance wherever the database keeps it; the id of its stored copy stays inside.
export const auditRecordSchema = createSelectSchema(audits).omit({ ruleProvenanceId: true }).extend({
  sessionId: z.string().nullable().describe("The session the audit belongs to; null for an audit the repository owns, of a change in another worktree that no session made."),
  repositoryId: z.string().nullable().describe("Of an audit the repository owns, the repository's key; null for a session's."),
  recordedBy: z.string().nullable().describe("Of an audit the repository owns, the session whose evaluator ran it; null for a session's."),
  paths: z.array(z.string()),
  violations: z.array(codeRuleViolationSchema),
  failures: z.array(auditFailureSchema),
  failureCount: z.number().int().nonnegative().describe("Every failure the audit had: those `failures` lists, and those it leaves out."),
  unlistedFailureCount: z.number().int().nonnegative().describe("Failures past the listed ones (at most 20 besides blind files, scan stops and one per rule that ran out of work)."),
  blindFiles: z.array(z.string()),
  blindFileCount: z.number().int().nonnegative(),
  catalogRevision: z.string().nullable(),
  ruleRevisions: z.array(z.object({ ruleId: z.string(), revision: z.number().int() }).strict()),
});
export const auditControlsResponseSchema = auditControlsSchema.extend({
  repositoryAuditRecord: auditRecordSchema.extend({}).nullable(),
});

export const auditQuerySchema = z.object({
  auditId: z.string().min(1).optional(),
  sessionId: z.string().min(1).optional(),
  status: z.enum(AUDIT_STATUSES).optional(),
  scope: z.enum(AUDIT_SCOPES).optional(),
  mode: z.enum(AUDIT_MODES).optional().describe("Audit mode (steer, log), not the mode of the rules it ran."),
  origin: z.enum(AUDIT_ORIGINS).optional().describe("ongoing: a session's own edits; scan: one someone asked for, which the dashboard leaves out."),
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
    scope: z.enum(AUDIT_SCOPES),
    status: z.enum(AUDIT_STATUSES),
    startedAt: z.number(),
    finishedAt: z.number().nullable(),
    revision: z.number().int().nonnegative(),
    catalogRevision: z.string().nullable(),
    ruleRevisionCount: z.number().int().nonnegative(),
    violationCount: z.number().int().nonnegative(),
    failureCount: z.number().int().nonnegative().describe("Every failure the audit had, listed or not."),
    unlistedFailureCount: z.number().int().nonnegative(),
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

/** `fields` without the ones that are undefined: a query takes an absent filter as no filter, never as `undefined`. */
function definedOnly<T extends object>(fields: { [K in keyof T]-?: T[K] | undefined }): T {
  return Object.fromEntries(Object.entries(fields).filter(([, value]) => value !== undefined)) as T;
}

function auditQuery(input: z.infer<typeof auditQuerySchema>): AuditQuery {
  const { auditId, sessionId, status, scope, mode, origin, limit, offset } = input;
  return definedOnly<AuditQuery>({ auditId, sessionId, status, scope, mode, origin, limit, offset });
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


const sessionNotFound = (sessionId: string): ApiError => new ApiError(404, "session-not-found", `session ${sessionId} does not exist`);

/**
 * The session `sessionId` names, when `caller` may read it: a person any recorded session, a coding agent only one
 * recorded in its own repository ({@link historyScope}). Another repository's session reads as one that does not
 * exist, so session ids are no oracle across repositories.
 */
function readableSession(stores: RerailApiStores, caller: ApiCaller, sessionId: string): StoredSessionSummary | undefined {
  const ids = historyScope(stores, caller, undefined);
  const session = stores.reader.session(sessionId);
  if (ids === undefined) return session;
  return session?.repositoryId != null && ids.includes(session.repositoryId) ? session : undefined;
}

function requireSession(stores: RerailApiStores, caller: ApiCaller, sessionId: string): void {
  if (readableSession(stores, caller, sessionId) === undefined) throw sessionNotFound(sessionId);
}

/** A coding agent's call on a session it may not read answers as {@link requireSession}; a person's goes through as ever. */
function guardSession(stores: RerailApiStores, caller: ApiCaller, sessionId: string): void {
  if (caller.kind === "agent") requireSession(stores, caller, sessionId);
}

function sessionResponse(stores: RerailApiStores, caller: ApiCaller, sessionId: string): z.infer<typeof sessionResponseSchema> | null {
  const session = readableSession(stores, caller, sessionId);
  return session === undefined ? null
    : { session, hud: stores.reader.hud(sessionId), audit: stores.audits.latestSummary(sessionId) ?? null };
}
const sessionId = z.object({ sessionId: z.string().min(1) });

/**
 * `operation` with the start state of the folder its `repository` names read also under `spelling`, the folder as a
 * Windows host spelt it ({@link RerailApiStores.repositoryState}); unchanged without one, or for any other folder.
 */
function readingSpelling(operation: Operation): Operation {
  return { ...operation, handler: (stores, input, caller) => operation.handler(spelledStores(stores, input), input, caller) };
}

/** `stores` whose start-state read of `repository` passes `spelling` on; `stores` itself where it is no absolute path. */
function spelledStores(stores: RerailApiStores, { repository, spelling }: { repository?: unknown; spelling?: unknown }): RerailApiStores {
  if (typeof spelling !== "string" || !isAbsolute(spelling)) return stores;
  const repositoryState = (workspace: string) => stores.repositoryState(workspace, workspace === repository ? spelling : undefined);
  return new Proxy(stores, { get: (target, key) => key === "repositoryState" ? repositoryState : boundTo(target, Reflect.get(target, key)) });
}

/** `value` called on `target`, as a property read of `target` gives it: stores keep private fields a proxy cannot reach. */
function boundTo(target: object, value: unknown): unknown {
  return typeof value === "function" ? (value as (...parameters: unknown[]) => unknown).bind(target) : value;
}

const repositoryFilter = {
  repository: z.string().min(1).optional().describe("Only this repository's history (its key or an alias); machine-wide without it. A coding agent reads only its own repository's."),
};

const sinceParameter = countParameter.describe("Epoch milliseconds; only activity at or after this instant is counted. 0 means since install.");
const PAGE_LIMIT = 50;
const pageLimitParameter = countParameter.min(1).max(200).optional()
  .describe(`Maximum rows to return; defaults to ${PAGE_LIMIT} and is capped at 200.`);
const cursorParameter = z.string().min(1).optional().describe("The `nextCursor` of the previous page; omit it for the first page.");
const nextCursorSchema = z.string().nullable().describe("Pass as `cursor` for the next page; null on the last page.");

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
  declinedViolations: z.number().int().nonnegative().describe("Of distinctViolations, those of a steer the coding agent declined with `rerail feedback`. A steer with no note is not known to be followed: an agent can decline without recording why."),
  declinedLoggedViolations: z.number().int().nonnegative().describe("Of declinedViolations, those also in loggedViolations (recorded in Log mode by another audit in range). Counting each violation once, as Log only, steered or declined, a declined one is declined: Log only is loggedViolations less these, steered the rest."),
  rerails: z.number().int().nonnegative(),
  ...inactiveField,
  removed: z.literal(true).optional().describe("Set on a rule no longer among the rules reported (taken out of the repository, or no longer installed or in the library) that has violations in range: listed for its history, with nothing to turn off."),
}).strict();

type RuleActivity = z.infer<typeof ruleActivitySchema>;

/** What `rules_activity` reads for a range: each rule's audit totals, steer deliveries and declined violations. */
interface RuleActivityCounts {
  activity: ReturnType<RerailApiStores["audits"]["ruleActivity"]>;
  rerails: ReadonlyMap<string, number>;
  declined: ReadonlyMap<string, DeclinedViolations>;
}

/**
 * A catalog rule's activity. Every rule is reported, so a caller never has to infer absence from a missing key; a
 * conversation rule writes no audit (so nothing of it is steered or declined) and an inactive code rule is never
 * evaluated, so neither is measured.
 */
function catalogRuleActivity(
  { id, kind, rule, notRun }: ReturnType<typeof activityRules>[number],
  counts: RuleActivityCounts,
): RuleActivity {
  const { inactive } = inactiveOf(kind, rule, notRun);
  const totals = kind === "conversation" ? undefined : counts.activity.get(id);
  return {
    measured: kind !== "conversation" && inactive === undefined,
    distinctViolations: totals?.distinctViolations ?? 0,
    sessions: totals?.sessions ?? 0,
    lastSeenAt: totals?.lastSeenAt ?? null,
    audits: totals?.audits ?? 0,
    loggedViolations: totals?.loggedViolations ?? 0,
    ...declinedOf(counts.declined, id),
    rerails: kind === "conversation" ? 0 : counts.rerails.get(id) ?? 0,
    ...(inactive === undefined ? {} : { inactive }),
  };
}

/** A rule taken out since keeps its violations: History still lists it, marked removed, while it has some in range. */
function addRemovedRuleActivity(rules: Record<string, RuleActivity>, counts: RuleActivityCounts): void {
  for (const [id, totals] of counts.activity) {
    if (rules[id] !== undefined || totals.distinctViolations === 0) continue;
    rules[id] = { measured: true, ...totals, ...declinedOf(counts.declined, id), rerails: counts.rerails.get(id) ?? 0, removed: true };
  }
}

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
  sessionId: z.string().nullable().describe("The session whose work the violation is in; null for one the repository's own audit recorded: a change in another worktree that no session made."),
  auditId: z.string(),
  at: z.number().int().describe("Start time of the audit that reported the violation."),
  host: z.string().nullable().describe("The coding agent of the violation's session; null when the session is not recorded, or there is none."),
  logged: z.boolean().describe("Rerail recorded the violation without telling the coding agent: the audit ran in Log mode, or the rule was in Log mode then. `mode` is the rule's mode alone."),
  declinedReasons: z.array(z.string()).describe("The reasons the coding agent recorded with `rerail feedback` for declining the steer of this violation's audit and rule, oldest first; empty when no note declined it, and always for a logged violation."),
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
  rerailData: z.object({
    access: z.enum(["kept-out", "file-tools", "open"]).describe("kept-out: neither its commands nor its file tools can read Rerail's data; file-tools: its file tools cannot, its shell commands can; open: nothing keeps it out."),
    why: z.string().describe("One sentence for a person saying why."),
    fix: z.string().nullable().describe("One sentence for a person saying what they can do on this computer to keep the rest of its tools out too; null when Rerail knows of nothing."),
  }).strict().describe("Whether this coding agent's own tools can read Rerail's data (the Rerail home and the content cache), as far as its configuration on this computer shows. Rerail never changes it."),
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
    summary: "Set compact or silent presentation for a repository and harness. Empty keys select database-local defaults; enforcement is unchanged. People only: the status line is the person's.",
    body: presentationPreferenceSchema,
    output: z.object({ preference: presentationPreferenceSchema }),
    handler: (stores, input, caller) => {
      if (caller.kind === "agent") throw new ApiError(403, "agent-not-permitted", "only a person can show or hide their status line, in the dashboard's Settings");
      return { preference: stores.preferences().upsert(input) };
    },
  }),
  defineOperation({
    id: "presentation_preferences_remove",
    method: "DELETE",
    path: "/api/presentation-preferences",
    summary: "Remove a presentation override so the next matching database-local default applies. People only.",
    query: presentationPreferenceKeySchema,
    output: z.object({ removed: z.number().int().nonnegative() }),
    handler: (stores, input, caller) => {
      if (caller.kind === "agent") throw new ApiError(403, "agent-not-permitted", "only a person can show or hide their status line, in the dashboard's Settings");
      return { removed: stores.preferences().remove(input.repository, input.host) };
    },
  }),
  readingSpelling(defineOperation({
    id: "presentation_get",
    method: "GET",
    path: "/api/presentation",
    summary: "Resolve repository+harness, repository, harness, database, then compact presentation; optionally read exact-session status without running an audit.",
    query: z.object({
      repository: z.string().min(1).refine(isAbsolute, "Repository must be an absolute path."),
      host: z.enum(PRESENTATION_HOSTS).exclude([""]),
      sessionId: z.string().min(1).optional().describe("Exact session for passive daemon health and compact status."),
      onDemand: booleanParameter.optional().describe("Read session status even when ambient presentation is silent."),
      spelling: z.string().optional().describe("On Windows, `repository` as the host spelt it, where that differs: read only for a start or stop Rerail 0.1.4 saved under it."),
    }).strict(),
    output: presentationResponseSchema,
    handler: (stores, input, caller) => {
      let session = input.sessionId === undefined ? null : sessionResponse(stores, caller, input.sessionId);
      let repository = input.repository;
      // A coding agent asks about presence where it works: its repository, or a checkout inside its own.
      if (caller.kind === "agent") {
        const own = callerRepositoryKey(stores, caller);
        const checkout = stores.checkouts()(repository);
        const inside = caller.checkout !== undefined && checkout !== undefined && within(caller.checkout, checkout.checkoutPath) !== undefined;
        if (own === undefined || (!inside && checkout?.repositoryKey !== own)) {
          throw new ApiError(403, "agent-not-permitted", `a coding agent reads only in the repository it works in${own === undefined ? "" : `, ${own}`}`);
        }
      }
      if (session !== null) {
        try {
          // A host may report a directory below its launch root; presence and identity follow the session's canonical cwd.
          const cwd = checkoutRealpath(session.session.cwd);
          if (session.session.host !== input.host || !withinCheckout(cwd, checkoutRealpath(input.repository))) session = null;
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
  })),
  defineOperation({
    id: "rules_activity",
    method: "GET",
    path: "/api/rules/activity",
    summary: "Report what each rule actually caught since a given time: distinct violations, sessions, last sighting, how many audits evaluated it, and how many steering deliveries it drove. A rule taken out of the repository is listed, with `removed: true`, while it has violations in range. Use it to find rules worth disabling or rules that never fire.",
    query: z.object({ since: sinceParameter, ...repositoryFilter }).strict(),
    output: z.object({
      since: z.number().int().nonnegative(),
      rules: z.record(z.string(), ruleActivitySchema),
      sessions: z.object({
        started: z.number().int().nonnegative().describe("Sessions that started at or after `since`."),
        withViolations: z.number().int().nonnegative().describe("Distinct sessions with a violation reported at or after `since`, in any mode."),
      }),
    }),
    handler: (stores, input, caller) => {
      const ids = historyScope(stores, caller, input.repository);
      const counts: RuleActivityCounts = {
        activity: stores.audits.ruleActivity(input.since, ids),
        rerails: stores.reader.steersByRule(input.since, ids),
        declined: stores.reader.declinedByRule(input.since, ids),
      };
      const rules: Record<string, RuleActivity> = {};
      // A coding agent's rules are its own repository's, as its counts are: never the machine's whole library.
      for (const entry of activityRules(stores, callerRepositoryKey(stores, caller) ?? input.repository)) {
        rules[entry.id] = catalogRuleActivity(entry, counts);
      }
      addRemovedRuleActivity(rules, counts);
      return {
        since: input.since,
        rules,
        sessions: { started: stores.reader.sessionCount(input.since, ids), withViolations: stores.audits.sessionsWithViolations(input.since, ids) },
      };
    },
  }),
  defineOperation({
    id: "rule_packs_list",
    method: "GET",
    path: "/api/rule-packs",
    summary: "List every rule pack installed on this machine: what it covers, its version, its npm package and publisher (rerail, or a third party Rerail has not reviewed) and its rules, each with a title, a group and the mode it starts in, and every installed pack left out with the reason. With a checkout (an MCP call defaults to the calling agent's), that checkout's own rules in .rerail/rules are listed too, as the pack worktree (publisher worktree), and every rule document there left out with the reason. Add a pack to a repository with repository_attachments_set.",
    query: z.object({ checkout: targetShape.checkout }).strict(),
    output: rulePackListSchema,
    handler: (stores, input, caller) => {
      // A checkout named is one Rerail has seen; the calling agent's adds its own rules only when it is one.
      const checkout = input.checkout !== undefined ? checkoutAt(stores, input.checkout, false).checkout
        : caller.kind === "agent" && caller.checkout !== undefined ? stores.checkouts()(caller.checkout) : undefined;
      const catalog = checkoutRuleCatalog(stores, checkout);
      const approvals = readApprovals(stores.database());
      return {
        // Where a pack installed from disk came from is named by its kind; the path stays on this computer's CLI.
        packs: [...catalog.packs.values()].map(pack => {
          const { match: _match, localSource, rules, ...rest } = pack;
          const subject = packSubject(catalog, pack);
          return {
            ...rest,
            ...(localSource === undefined ? {} : { installedFrom: localSource.kind }),
            ...(subject === undefined ? {} : { approval: { subject: { ...subject }, approved: approvals.has(approvalKey(subject)) } }),
            rules: rules.map(rule => ({ ...rule, ...inactiveOf(rule.kind, catalog.documents.get(rule.id)?.rule, catalog.notRun.get(rule.id)) })),
          };
        }),
        skipped: [...catalog.skipped],
      };
    },
  }),
  defineOperation({
    id: "coding_agents_list",
    method: "GET",
    path: "/api/coding-agents",
    summary: "List the coding agents Rerail works with: whether each is on this computer, whether Rerail is connected to it, and whether its tools can read Rerail's data. Changes nothing.",
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
    handler: (stores, input, caller) => {
      requireSession(stores, caller, input.sessionId);
      return auditControlsResponse(stores, input.sessionId);
    },
  }),
  defineOperation({
    id: "audit_settings_update",
    method: "PATCH",
    path: "/api/sessions/{sessionId}/audit-settings",
    summary: "Update ongoing audit defaults for one session's canonical workspace. People only (rerail settings): they outlast the session, so a coding agent could otherwise stop its checkout's later sessions, the person's included, from being steered.",
    params: sessionId,
    body: auditSettingsPatchSchema,
    output: auditControlsResponseSchema,
    handler: (stores, input, caller) => {
      if (caller.kind === "agent") {
        throw new ApiError(403, "agent-not-permitted", "only a person changes a checkout's ongoing audit settings, with rerail settings --session <id>");
      }
      requireSession(stores, caller, input.sessionId);
      const { sessionId, ...patch } = input;
      stores.auditControls().updateSettings(sessionId, patch);
      return auditControlsResponse(stores, sessionId);
    },
  }),
  defineOperation({
    id: "audit_requests_create",
    method: "POST",
    path: "/api/sessions/{sessionId}/audit-requests",
    experiment: "REPOSITORY_SCAN",
    summary: internalTesting(experimentalSummary("REPOSITORY_SCAN", "Queue an explicit audit for one session. Touched-file audits require paths.")),
    params: sessionId,
    body: auditRequestInputSchema,
    output: auditRequestSchema,
    handler: (stores, input, caller) => {
      requireSession(stores, caller, input.sessionId);
      const { sessionId, ...request } = input;
      const created = stores.auditControls().requestAudit(sessionId, request);
      // A session the daemon suspended while it was quiet keeps its owner, so its scan queues; it opens again to run it
      // now, as an open session would. The answer does not wait for that: it reads the session's ledger back.
      if (created.status === "queued") stores.resumeSession?.(sessionId);
      return created;
    },
  }),
  defineOperation({
    id: "audit_requests_list",
    method: "GET",
    path: "/api/sessions/{sessionId}/audit-requests",
    experiment: "REPOSITORY_SCAN",
    summary: internalTesting(experimentalSummary("REPOSITORY_SCAN", "List explicit audit requests and the ids of the audits each one produced for one session.")),
    params: sessionId,
    output: z.object({ auditRequests: z.array(auditRequestSchema) }),
    handler: (stores, input, caller) => {
      requireSession(stores, caller, input.sessionId);
      return { auditRequests: stores.auditControls().listAuditRequests(input.sessionId) };
    },
  }),
  defineOperation({
    id: "repository_audits_list",
    method: "GET",
    path: "/api/repository-audits",
    experiment: "REPOSITORY_SCAN",
    summary: internalTesting(experimentalSummary("REPOSITORY_SCAN", "List one full repository audit baseline per historically scanned repository.")),
    query: repositoryAuditQuerySchema.extend(repositoryFilter),
    output: z.object({
      repositories: z.array(repositoryAuditSummarySchema),
      total: z.number().int().nonnegative(),
    }),
    handler: (stores, input, caller) => stores.audits.listRepositoryAudits(repositoryAuditQuery(input), historyScope(stores, caller, input.repository)),
  }),
  defineOperation({
    id: "repository_audit_entries_list",
    method: "GET",
    path: "/api/repository-audits/{auditId}/entries",
    experiment: "REPOSITORY_SCAN",
    summary: internalTesting(experimentalSummary("REPOSITORY_SCAN", "Query paginated violations and failures from one repository audit baseline.")),
    params: z.object({ auditId: z.string().min(1) }),
    query: repositoryAuditEntryQuerySchema,
    output: z.object({
      entries: z.array(repositoryAuditEntrySchema),
      total: z.number().int().nonnegative(),
    }),
    handler: (stores, input, caller) => {
      const ids = historyScope(stores, caller, undefined);
      // Another repository's audit reads as one that does not exist, as its sessions do.
      const readable = ids === undefined || stores.audits.list({ auditId: input.auditId, limit: 1 }, ids).total > 0;
      const page = readable ? stores.audits.listRepositoryAuditEntries(repositoryAuditEntryQuery(input)) : undefined;
      if (page === undefined) throw new ApiError(404, "repository-audit-not-found", `repository audit ${input.auditId} does not exist`);
      const view = violationView(caller);
      const root = stores.audits.workspaceRoots([input.auditId]).get(input.auditId);
      return {
        entries: page.entries.map(entry => entry.kind === "violation"
          ? { kind: "violation" as const, violation: view(codeRuleViolationSchema.parse(entry.payload), root) }
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
    handler: (stores, input, caller) => {
      const page = stores.audits.list(auditQuery(input), historyScope(stores, caller, input.repository));
      if (caller.kind === "person") return page;
      const view = violationView(caller);
      return { ...page, audits: page.audits.map(audit => ({ ...audit, violations: audit.violations.map(violation => view(violation, audit.workspaceRoot)) })) };
    },
  }),
  defineOperation({
    id: "violations_list",
    method: "GET",
    path: "/api/violations",
    summary: "List individual violations newest first, optionally for one rule or one session, with the session and audit that reported each. Use it to read what a rule actually flagged rather than how often.",
    query: z.object({
      since: sinceParameter,
      limit: pageLimitParameter,
      cursor: cursorParameter,
      ruleId: z.string().min(1).optional().describe("Restrict to one rule id."),
      sessionId: z.string().min(1).optional().describe("Restrict to one session."),
      ...repositoryFilter,
    }).strict(),
    output: z.object({ total: z.number().int().nonnegative(), violations: z.array(violationRecordSchema), nextCursor: nextCursorSchema }),
    handler: (stores, input, caller) => {
      const page = stores.audits.listViolationHistory({
        since: input.since,
        ...(input.limit === undefined ? {} : { limit: input.limit }),
        ...(input.cursor === undefined ? {} : { cursor: input.cursor }),
        ...(input.ruleId === undefined ? {} : { ruleId: input.ruleId }),
        ...(input.sessionId === undefined ? {} : { sessionId: input.sessionId }),
      }, historyScope(stores, caller, input.repository));
      const view = violationView(caller);
      const roots = stores.audits.workspaceRoots([...new Set(page.violations.map(v => v.auditId))]);
      const reasons = declineReasonsBySteer(stores.reader.placedDeclineNotes([...new Set(page.violations.flatMap(v => v.sessionId ?? []))]));
      return {
        total: page.total,
        violations: page.violations.map(({ violation: found, sessionId, auditId, at, host, logged }) => {
          const { id, ruleId, clauseId, code, mode, message, path, span, fix } = view(found, roots.get(auditId));
          const declinedReasons = logged ? [] : reasons.get(steerKey(auditId, ruleId)) ?? [];
          return { id, ruleId, clauseId, code, mode, message, path: path ?? null, span: span ?? null, fix: fix ?? null, sessionId, auditId, at, host, logged, declinedReasons };
        }),
        nextCursor: page.nextCursor,
      };
    },
  }),
  defineOperation({
    id: "sessions_list",
    method: "GET",
    path: "/api/sessions",
    summary: "List recorded harness sessions newest first, a page at a time, with their event and violation counts, optionally only those started since a given time.",
    query: z.object({
      since: sinceParameter.optional().describe("Epoch milliseconds; only sessions started at or after this instant."),
      limit: pageLimitParameter,
      cursor: cursorParameter,
      ...repositoryFilter,
    }).strict(),
    output: z.object({
      sessions: z.array(sessionSummarySchema.extend({
        violationCount: z.number().int().nonnegative().describe("Violations in the session's history, as `violations_list` with its `sessionId` counts them."),
        steeredViolations: z.number().int().nonnegative().describe("Of violationCount, those no audit recorded in Log mode (steered, or blocked)."),
        declinedViolations: z.number().int().nonnegative().describe("Of steeredViolations, those of a steer the coding agent declined, as `sessions_get` counts them in `declined`."),
        lastEventAt: z.number().int().nullable().describe("Epoch milliseconds the session's newest event was observed; null when it has none."),
      })),
      total: z.number().int().nonnegative(),
      nextCursor: nextCursorSchema,
    }),
    handler: (stores, input, caller) => stores.reader.sessionPage({
      limit: input.limit ?? PAGE_LIMIT,
      ...(input.since === undefined ? {} : { since: input.since }),
      ...(input.cursor === undefined ? {} : { cursor: input.cursor }),
    }, historyScope(stores, caller, input.repository)),
  }),
  defineOperation({
    id: "sessions_get",
    method: "GET",
    path: "/api/sessions/{sessionId}",
    summary: "Fetch one session with rule coverage, steering handoffs, compact audit status, and what its transcript says it missed.",
    params: sessionId,
    output: sessionResponseSchema.extend({
      gaps: z.object({
        droppedEvents: z.number().int().nonnegative().describe("Events the coding agent never delivered, from its `events-dropped` records."),
        uncheckedBoundaries: z.number().int().nonnegative().describe("Boundaries that answered before the session's shell-write check finished."),
        codeRulesPendingBoundaries: z.number().int().nonnegative().describe("Boundaries that answered before the session's code rules were running."),
        ignoredEdits: z.number().int().nonnegative().describe("Tool edits of files no rule checked because Git ignores them (`ignored-edit` events name each file and the ignore file, line and pattern)."),
        ignoreRuleChanges: z.number().int().nonnegative().describe("Changes to Git's ignore rules seen during the session that can make them ignore more: a pattern added, a negation (`!pattern`) removed, or, in a file with a negation, its patterns reordered or repeated. Each `ignore-rules-changed` event names the file and those patterns; comments, blank lines and duplicates in a file without a negation are not changes."),
        discoveryGaps: z.number().int().nonnegative().describe(`Places the session's shell-write check could not fully check, in the session's folder or the repository's other worktrees: a folder or file Rerail cannot read; a file over 1,000,000 bytes that a rule would read; a source or text file holding binary data, such as UTF-16 text; a walk that fails. Shell writes in such a place went unchecked. Each place is counted once in the session, by its path and its error's code, however many boundaries met it, across a reopened session and a daemon restart; its \`source-discovery-gap\` event holds the path, the reason and the first boundary, and is withheld from coding agents. The check records at most a bound of places in one session; this counts the places recorded, and \`discoveryGapsMore\` says whether it met more.`),
        discoveryGapsMore: z.boolean().describe("Whether the shell-write check met more places it could not fully check than the session's record takes: `discoveryGaps` counts those recorded, the first it met, and one more record says the rest went unrecorded."),
        discoveryGapPlaces: z.array(z.object({
          path: z.string().optional().describe("The place as the session folder names it (`.` for the folder itself, `../other/src` in another worktree); absent when a walk failed whole."),
          reason: z.string().describe("Why the check could not fully check it, as the walk was told."),
          boundary: z.string().describe("The boundary that first met it."),
          observedAt: z.number().int().describe("Epoch milliseconds it was first met."),
        })).max(DISCOVERY_GAP_PLACES).optional().describe(`The first ${DISCOVERY_GAP_PLACES} places \`discoveryGaps\` counts, in the order the session met them. A person's only: a coding agent gets none, as it gets no \`source-discovery-gap\` payload.`),
      }).describe("Over the whole transcript, however much of it a reader has loaded."),
      declined: z.object({
        violations: z.number().int().nonnegative().describe("Steered violations of a declined steer: its audit's violations of the rule, each counted once."),
        notes: z.array(z.object({
          ruleId: z.string(),
          reason: z.string().describe("The one line the coding agent gave the person for not following the steer."),
          recordedAt: z.number().int(),
          auditId: z.string().nullable().describe("The audit whose steer it declined: the latest in the session to steer the rule's violations and start at or before the note; null when none did."),
        })).describe("Oldest first."),
      }).describe("Steers the coding agent declined, from the notes it recorded with `rerail feedback`. A steer with no note is not known to be followed: an agent can decline without recording why."),
    }),
    handler: (stores, input, caller) => {
      const response = sessionResponse(stores, caller, input.sessionId);
      if (response === null) throw sessionNotFound(input.sessionId);
      const places = caller.kind === "person" ? { discoveryGapPlaces: [...stores.reader.discoveryGaps(input.sessionId, DISCOVERY_GAP_PLACES)] } : {};
      return { ...response, gaps: { ...stores.reader.gaps(input.sessionId), ...places }, declined: stores.reader.declines(input.sessionId) };
    },
  }),
  defineOperation({
    id: "session_events",
    method: "GET",
    path: "/api/sessions/{sessionId}/events",
    summary: `Page through one session's transcript events in sequence order, at most ${SESSION_EVENT_PAGE_LIMIT} per page: forward from \`after\`, or backward from \`before\` (the latest page is before a sequence past the end). A coding agent gets every tool output withheld, the input of a call that names a hidden or secret-like path, and the payload of an event kind Rerail does not know.`,
    params: sessionId,
    query: z.object({
      after: countParameter.optional().describe("Return events with a sequence strictly greater than this value."),
      before: countParameter.optional().describe("Return the last page of events with a sequence strictly less than this value."),
      limit: countParameter.optional(),
    }),
    output: z.object({
      events: z.array(eventRecordSchema),
      nextAfter: z.number().int().nonnegative(),
      pageLimit: z.literal(SESSION_EVENT_PAGE_LIMIT),
    }),
    handler: (stores, input, caller) => {
      guardSession(stores, caller, input.sessionId);
      const page = stores.reader.events(input.sessionId, {
        ...(input.after === undefined ? {} : { after: input.after }),
        ...(input.before === undefined ? {} : { before: input.before }),
        ...(input.limit === undefined ? {} : { limit: input.limit }),
      });
      const last = page.at(-1);
      // A call's paths are judged in the session's folder too, where a file that links to a secret counts as one.
      const cwd = caller.kind === "person" ? undefined : stores.reader.session(input.sessionId)?.cwd;
      return {
        events: caller.kind === "person" ? [...page] : page.map(event => ({ ...event, payload: agentTranscriptPayload(event.kind, event.payload, cwd) })),
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
    handler: (stores, input, caller) => {
      guardSession(stores, caller, input.sessionId);
      return { workLinks: [...stores.reader.workLinks(input.sessionId)] };
    },
  }),
  defineOperation({
    id: "work_links_add",
    method: "PUT",
    path: "/api/sessions/{sessionId}/work-links",
    summary: "Link a session to an external reference. Re-linking the same kind and reference updates its url.",
    params: sessionId,
    body: workLinkInputSchema,
    output: z.object({ workLinks: z.array(workLinkRecordSchema) }),
    handler: (stores, input, caller) => {
      guardSession(stores, caller, input.sessionId);
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
  ...mergeAssistOperations,
];

/** The flat argument object an MCP tool takes: path params, query, and body merged. */
export function operationInputShape(operation: Operation): z.ZodRawShape {
  return { ...operation.params?.shape, ...operation.query?.shape, ...operation.body?.shape };
}
