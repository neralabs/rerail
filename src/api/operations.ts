import { realpathSync } from "node:fs";
import { isAbsolute } from "node:path";
import { createInsertSchema, createSelectSchema } from "drizzle-zod";
import { z } from "zod";
import type { ExecutablePolicyFinding, PolicyFindingFix, PolicySourceSpan } from "../executable-policy.js";
import type { PolicyExecutionFailure } from "../policy-execution.js";
import type { PolicyExecutionRunReader, PolicyRunQuery, RepositoryDiagnosticEntryQuery, RepositoryDiagnosticQuery } from "../policy-execution-store.js";
import { withinCheckout } from "../checkout-control.js";
import type { RerailDatabase } from "../database.js";
import { listRepositories, repositorySetUp, setCheckoutState, startRepository, type RepositoryStartState } from "../repository-start.js";
import {
  policyRuns,
  presentationPreferences,
  PRESENTATION_HOSTS,
  RULE_APPLICABILITIES,
  RULE_KINDS,
  RULE_ORIGINS,
  events,
  workLinks,
} from "../schema.js";
import {
  policyControlsSchema,
  policySettingsPatchSchema,
  repositoryDiagnosticSchema,
  scanInputSchema,
  scanRequestSchema,
} from "../scan-control-schema.js";
import type { SqliteScanControlStore } from "../scan-control.js";
import { PolicyCompileError, PolicyKernelUnavailableError } from "../policy-kernel-check.js";
import { policyRuleSchema } from "../policy-schema.js";
import { RULE_MODES } from "../rule-mode.js";
import { kernelRegexIssues } from "../rule-regexes.js";
import { cosineRuleSchema, SAMPLE_BOUNDARIES } from "../rule-schema.js";
import { QuarantinedGlobalRuleError, type ScopedRuleCatalog, type ScopedRuleQuery } from "../scoped-rule-catalog.js";
import { cachedRecommendRulePacks, loadRulePacks, rulePackIdOf } from "../rule-packs.js";
import { rulePackListSchema, rulePackRecommendationsSchema } from "./rule-pack-contracts.js";
import { TRANSCRIPT_EVENT_PAGE_LIMIT, type SqliteTranscriptReader, type SqliteTranscriptStore } from "../transcript-store.js";
import type { SqlitePresentationPreferenceStore } from "../presentation-preference-store.js";
import { presentationDaemonSchema, presentationResponseSchema, sessionResponseSchema, sessionSummarySchema } from "./session-contracts.js";

/**
 * One operation definition feeds every consumer: the daemon's HTTP routes and
 * OpenAPI document, the MCP tool list, and (through OpenAPI) the generated web
 * client. `params`, `query`, and `body` are split for HTTP; MCP tools take
 * their union as one flat argument object.
 */
export interface RerailApiStores {
  /** Read-only view; tolerates a database that does not exist yet. */
  readonly reader: SqliteTranscriptReader;
  /** Machine catalog resolved for the daemon's repository/project context. */
  rules(): ScopedRuleCatalog;
  /** Writable session/event/work-link store; opening it creates the database. */
  transcripts(): SqliteTranscriptStore;
  /** Durable per-session scan defaults and explicit scan requests. */
  scanControls(): SqliteScanControlStore;
  /** Read-only execution history; remains empty before any writer creates the database. */
  readonly executions: PolicyExecutionRunReader;
  /** Daemon-owned presentation settings in the project database. */
  preferences(): SqlitePresentationPreferenceStore;
  /** Writable handle for state with no store of its own: whether Rerail is started per repository. */
  database(): RerailDatabase;
  /** Whether Rerail is on in the checkout a workspace is in, over one held connection with cached identities. */
  repositoryState(workspace: string): RepositoryStartState;
  health(): z.infer<typeof presentationDaemonSchema>;
}

export type OperationMethod = "GET" | "POST" | "PUT" | "PATCH" | "DELETE";

export interface OperationDefinition<
  Params extends z.ZodRawShape,
  Query extends z.ZodRawShape,
  Body extends z.ZodRawShape,
  Output extends z.ZodType,
> {
  /** MCP tool name and OpenAPI operationId: `[a-z0-9_]+`. */
  id: string;
  method: OperationMethod;
  /** OpenAPI-style path, e.g. `/api/rules/{id}`. */
  path: string;
  summary: string;
  params?: z.ZodObject<Params>;
  query?: z.ZodObject<Query>;
  body?: z.ZodObject<Body>;
  output: Output;
  handler: (
    stores: RerailApiStores,
    input: z.infer<z.ZodObject<Params>> & z.infer<z.ZodObject<Query>> & z.infer<z.ZodObject<Body>>,
  ) => z.infer<Output>;
}

export type Operation = OperationDefinition<z.ZodRawShape, z.ZodRawShape, z.ZodRawShape, z.ZodType>;

function defineOperation<
  Params extends z.ZodRawShape,
  Query extends z.ZodRawShape,
  Body extends z.ZodRawShape,
  Output extends z.ZodType,
>(operation: OperationDefinition<Params, Query, Body, Output>): Operation {
  return operation as unknown as Operation;
}

/** Thrown by handlers; the HTTP adapter maps it to a status, the MCP adapter to an error result. */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;

  constructor(status: number, code: string, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

/** Query strings arrive as text; MCP arguments arrive typed. Accept both. */
const booleanParameter = z.union([z.boolean(), z.stringbool()]);
const countParameter = z.coerce.number().int().nonnegative();

/** Filters shared by listing and by filter-driven bulk updates. */
export const ruleFilterSchema = z
  .object({
    kind: z.enum(RULE_KINDS).optional(),
    enabled: booleanParameter.optional(),
    applicability: z.enum(RULE_APPLICABILITIES).optional(),
    q: z.string().optional().describe("Case-insensitive substring over id, reference/description, and action content or clause bodies."),
    source: z.string().min(1).optional().describe("Conversation sample source (user-input, assistant-text, tool:<name>)."),
    boundary: z.enum(SAMPLE_BOUNDARIES).optional(),
    mode: z.enum(RULE_MODES).optional().describe("Rule mode (log, steer, block); matches file and conversation rules."),
  })
  .strict();

function ruleQuery(filter: z.infer<typeof ruleFilterSchema>): ScopedRuleQuery {
  return {
    ...(filter.kind === undefined ? {} : { kind: filter.kind }),
    ...(filter.enabled === undefined ? {} : { enabled: filter.enabled }),
    ...(filter.applicability === undefined ? {} : { applicability: filter.applicability }),
    ...(filter.q === undefined ? {} : { text: filter.q }),
    ...(filter.source === undefined ? {} : { source: filter.source }),
    ...(filter.boundary === undefined ? {} : { boundary: filter.boundary }),
    ...(filter.mode === undefined ? {} : { mode: filter.mode }),
  };
}

const ruleOriginSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("global") }).strict(),
  z.object({ kind: z.literal("project"), projectId: z.string().min(1) }).strict(),
]);

const quarantinedRuleSchema = z.object({
  ruleId: z.string(),
  kind: z.enum(RULE_KINDS),
  owner: z.string().describe("global, project:<id>, or the per-project store's row scope."),
  reason: z.string(),
}).strict();

const storedRuleBase = {
  enabled: z.boolean(),
  applicability: z.enum(RULE_APPLICABILITIES),
  origin: ruleOriginSchema,
  catalogOrigin: z.enum(RULE_ORIGINS),
  revision: z.number().int().positive(),
  createdAt: z.number().int(),
  updatedAt: z.number().int(),
  sourceDigest: z.string().nullable(),
  payloadHash: z.string(),
};

export const storedRuleSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("file"), rule: policyRuleSchema, ...storedRuleBase }).strict(),
  z.object({ kind: z.literal("conversation"), rule: cosineRuleSchema, ...storedRuleBase }).strict(),
]);

const packIdField = { packId: z.string().nullable().describe("The rule pack this rule comes from; null for rules in no pack.") };
const listedRuleSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("file"), rule: policyRuleSchema, ...storedRuleBase, ...packIdField }).strict(),
  z.object({ kind: z.literal("conversation"), rule: cosineRuleSchema, ...storedRuleBase, ...packIdField }).strict(),
]);

export const ruleUpsertSchema = z
  .object({
    kind: z.enum(RULE_KINDS),
    rule: z.union([policyRuleSchema, cosineRuleSchema]),
    enabled: z.boolean().optional(),
    applicability: z.enum(RULE_APPLICABILITIES).optional(),
    /** Conversation rules only; file rules carry `rule.revision`. */
    revision: z.number().int().positive().optional(),
  })
  .strict()
  .superRefine((value, context) => {
    const looksPolicy = "clauses" in value.rule;
    if ((value.kind === "file") !== looksPolicy) {
      context.addIssue({ code: "custom", path: ["rule"], message: `rule body does not match kind ${value.kind}` });
    }
    if (value.kind === "file" && value.revision !== undefined) {
      context.addIssue({ code: "custom", path: ["revision"], message: "file rules take their revision from rule.revision" });
    }
    if (value.kind === "conversation" && !("clauses" in value.rule)) {
      for (const issue of kernelRegexIssues(value.rule)) {
        context.addIssue({ code: "custom", path: ["rule", ...issue.path], message: issue.message });
      }
    }
  });

export const eventRecordSchema = createSelectSchema(events)
  .omit({ payloadJson: true, payloadV8: true })
  .extend({ payload: z.unknown() });

export const workLinkRecordSchema = createSelectSchema(workLinks);
export const workLinkInputSchema = createInsertSchema(workLinks).pick({ kind: true, reference: true, url: true });

const ruleId = z.object({ id: z.string().min(1) });

const policySourceSpanSchema: z.ZodType<PolicySourceSpan> = z.object({
  startByte: z.number().int(),
  endByte: z.number().int(),
  startLine: z.number().int(),
  startColumn: z.number().int(),
  endLine: z.number().int(),
  endColumn: z.number().int(),
}).strict();

const policyFindingFixSchema: z.ZodType<PolicyFindingFix> = z.object({
  span: policySourceSpanSchema,
  text: z.string().describe("Replacement text for the span. A suggestion only; Rerail never applies it."),
}).strict();

const executablePolicyFindingSchema: z.ZodType<ExecutablePolicyFinding> = z.object({
  id: z.string(),
  ruleId: z.string(),
  clauseId: z.string(),
  code: z.string(),
  mode: z.enum(RULE_MODES),
  message: z.string(),
  path: z.string().optional(),
  span: policySourceSpanSchema.optional(),
  captures: z.record(z.string(), z.array(policySourceSpanSchema)),
  variables: z.record(z.string(), z.union([z.boolean(), z.number(), z.string()])),
  fix: policyFindingFixSchema.optional(),
}).strict();

const policyExecutionFailureSchema: z.ZodType<PolicyExecutionFailure> = z.object({
  ruleId: z.string().exactOptional(),
  clauseId: z.string(),
  code: z.enum(["budget-exceeded", "missing-capability", "source-incomplete", "source-partial", "adapter-error"]),
  message: z.string(),
  path: z.string().optional(),
  stage: z.enum(["read", "parse", "evaluate", "scan", "scope", "persistence"]).optional(),
}).strict();

export const policyRunRecordSchema = createSelectSchema(policyRuns).extend({
  paths: z.array(z.string()),
  violations: z.array(executablePolicyFindingSchema),
  failures: z.array(policyExecutionFailureSchema),
  catalogRevision: z.string().nullable(),
  ruleRevisions: z.array(z.object({ ruleId: z.string(), revision: z.number().int() }).strict()),
});
export const policyControlsResponseSchema = policyControlsSchema.extend({
  repositoryAuditRecord: policyRunRecordSchema.extend({}).nullable(),
});

export const policyRunQuerySchema = z.object({
  auditId: z.string().min(1).optional(),
  sessionId: z.string().min(1).optional(),
  status: z.enum(["running", "completed", "failed", "interrupted", "superseded"]).optional(),
  scope: z.enum(["repository", "touched-files", "ai-output"]).optional(),
  mode: z.enum(["steer", "log"]).optional().describe("Audit mode (steer, log), not the mode of the rules it ran."),
  limit: countParameter.min(1).max(200).optional().describe("Maximum records to return; defaults to 50 and is capped at 200."),
  offset: countParameter.optional().describe("Number of matching records to skip; defaults to 0."),
}).strict();
const repositoryDiagnosticQuerySchema = z.object({
  workspaceRoot: z.string().min(1).optional(),
  status: repositoryDiagnosticSchema.shape.status.optional(),
  limit: countParameter.min(1).max(200).optional(),
  offset: countParameter.optional(),
}).strict();
const repositoryDiagnosticSummarySchema = z.object({
  workspaceRoot: z.string(),
  repositoryAudit: repositoryDiagnosticSchema.nullable(),
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
const repositoryDiagnosticEntryQuerySchema = z.object({
  kind: z.enum(["violation", "failure"]).optional(),
  mode: z.enum(RULE_MODES).optional().describe("Mode of the rule that produced a violation; failures never match."),
  code: z.string().optional(),
  path: z.string().optional(),
  q: z.string().optional(),
  limit: countParameter.min(1).max(200).optional(),
  offset: countParameter.optional(),
}).strict();
const repositoryDiagnosticEntrySchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("violation"), violation: executablePolicyFindingSchema }).strict(),
  z.object({ kind: z.literal("failure"), failure: policyExecutionFailureSchema }).strict(),
]);

function policyRunQuery(input: z.infer<typeof policyRunQuerySchema>): PolicyRunQuery {
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
function repositoryDiagnosticQuery(input: z.infer<typeof repositoryDiagnosticQuerySchema>): RepositoryDiagnosticQuery {
  return {
    ...(input.workspaceRoot === undefined ? {} : { workspaceRoot: input.workspaceRoot }),
    ...(input.status === undefined ? {} : { status: input.status }),
    ...(input.limit === undefined ? {} : { limit: input.limit }),
    ...(input.offset === undefined ? {} : { offset: input.offset }),
  };
}
function repositoryDiagnosticEntryQuery(
  input: z.infer<typeof repositoryDiagnosticEntryQuerySchema> & { auditId: string },
): RepositoryDiagnosticEntryQuery {
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
function policyControlsResponse(stores: RerailApiStores, sessionId: string): z.infer<typeof policyControlsResponseSchema> {
  const controls = stores.scanControls().getSettings(sessionId);
  const auditId = controls.repositoryAudit?.auditId;
  return {
    ...controls,
    repositoryAuditRecord: auditId === null || auditId === undefined ? null : stores.executions.get(auditId) ?? null,
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
    : { session, hud: stores.reader.hud(sessionId), audit: stores.executions.latestSummary(sessionId) ?? null };
}
const sessionId = z.object({ sessionId: z.string().min(1) });
const projectRepository = z.object({
  projectId: z.string().min(1),
  repositoryId: z.string().min(1),
});
const ruleContextSchema = z.object({
  repositoryId: z.string().min(1),
  workspaceRoot: z.string().min(1),
  projectId: z.string().min(1),
  projectName: z.string().min(1),
});

const repositoryStartSchema = z.object({
  repositoryId: z.string().min(1),
  name: z.string(),
  root: z.string().describe("The checkout Rerail last ran in; empty when only the start state is known."),
  state: z.enum(["started", "stopped", "never"]).describe(
    "The main checkout's on/off, which every worktree without its own setting follows. stopped also when the repository was set up only in a worktree; never only when no checkout of it was ever started.",
  ),
  startedAt: z.number().int().nullable(),
  stoppedAt: z.number().int().nullable(),
}).strict();
const repositoryParameter = z.object({ repositoryId: z.string().min(1) });

function knownRepository(stores: RerailApiStores, repositoryId: string): z.infer<typeof repositoryStartSchema> {
  const repository = listRepositories(stores.database()).find(candidate => candidate.repositoryId === repositoryId);
  if (repository === undefined) throw new ApiError(404, "repository-not-found", `repository ${repositoryId} is not known to Rerail`);
  return repository;
}

const sinceParameter = countParameter.describe("Epoch milliseconds; only activity at or after this instant is counted. 0 means since install.");

/**
 * What the catalog knows about one rule's effect over a time range. Conversation rules never write an
 * audit, so they carry no evidence either way: `measured: false` says so rather than
 * reporting a zero that reads as "never triggered".
 */
const ruleActivitySchema = z.object({
  measured: z.boolean(),
  distinctViolations: z.number().int().nonnegative().describe("Distinct (rule, clause, path, span, code) violations; one unfixed problem seen by six audits counts once."),
  sessions: z.number().int().nonnegative(),
  lastSeenAt: z.number().int().nullable(),
  audits: z.number().int().nonnegative().describe("Distinct audits in range that compiled this rule, so a zero violation count over a positive number of audits is evidence rather than silence."),
  rerails: z.number().int().nonnegative(),
}).strict();

const findingRecordSchema = z.object({
  id: z.string(),
  ruleId: z.string(),
  clauseId: z.string(),
  code: z.string(),
  mode: z.enum(RULE_MODES),
  message: z.string(),
  path: z.string().nullable(),
  span: policySourceSpanSchema.nullable(),
  fix: policyFindingFixSchema.nullable(),
  sessionId: z.string(),
  auditId: z.string(),
  at: z.number().int().describe("Start time of the audit that reported the violation."),
}).strict();

const repositoryPathSchema = z.string().refine(value => value === "" || isAbsolute(value), "Repository must be an absolute path or empty for the database default.");
export const presentationPreferenceSchema = createSelectSchema(presentationPreferences).extend({
  repository: repositoryPathSchema,
}).strict();
const presentationPreferenceKeySchema = presentationPreferenceSchema.pick({ repository: true, host: true });

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
          // A harness may report a directory below its launch root; presence and identity follow the session's canonical cwd.
          const cwd = realpathSync(session.session.cwd);
          if (session.session.host !== input.host || !withinCheckout(cwd, realpathSync(input.repository))) session = null;
          else repository = cwd;
        } catch { session = null; }
      }
      const presence = stores.preferences().effective(repository, input.host);
      const repositoryState = stores.repositoryState(repository);
      if (input.sessionId === undefined) return { presence, repositoryState };
      const daemon = presentationDaemonSchema.parse(stores.health());
      if (presence === "silent" && !input.onDemand) session = null;
      return { presence, repositoryState, context: { daemon, session } };
    },
  }),
  defineOperation({
    id: "rule_context_get",
    method: "GET",
    path: "/api/rule-context",
    summary: "Show the stable repository identity and project used to resolve effective rules.",
    output: z.object({ context: ruleContextSchema }),
    handler: stores => ({ context: stores.rules().context() }),
  }),
  defineOperation({
    id: "project_repository_assign",
    method: "PUT",
    path: "/api/projects/{projectId}/repositories/{repositoryId}",
    summary: "Assign one stable repository identity to a project.",
    params: projectRepository,
    body: z.object({ projectName: z.string().min(1) }).strict(),
    output: projectRepository,
    handler: (stores, input) => {
      stores.rules().assignRepository(input.projectId, input.projectName, input.repositoryId);
      return { projectId: input.projectId, repositoryId: input.repositoryId };
    },
  }),
  defineOperation({
    id: "project_repository_remove",
    method: "DELETE",
    path: "/api/projects/{projectId}/repositories/{repositoryId}",
    summary: "Remove one repository from a project; its own daemon falls back to a repository-local project.",
    params: projectRepository,
    output: z.object({ removed: z.boolean() }),
    handler: (stores, input) => ({ removed: stores.rules().removeRepository(input.projectId, input.repositoryId) }),
  }),
  defineOperation({
    id: "repositories_list",
    method: "GET",
    path: "/api/repositories",
    summary: "List the repositories Rerail knows, each Started, Stopped, or never set up.",
    output: z.object({ repositories: z.array(repositoryStartSchema) }),
    handler: stores => ({ repositories: listRepositories(stores.database()) }),
  }),
  defineOperation({
    id: "repository_start",
    method: "POST",
    path: "/api/repositories/{repositoryId}/start",
    summary: "Start Rerail in a repository's main checkout and the worktrees that follow it. A repository never set up is started with /rerail-start in a session there.",
    params: repositoryParameter,
    output: z.object({ repository: repositoryStartSchema }),
    handler: (stores, input) => {
      const database = stores.database();
      if (!repositorySetUp(database, input.repositoryId)) {
        throw new ApiError(409, "repository-not-set-up", `Rerail has not been set up in ${input.repositoryId}; run /rerail-start in a session there`);
      }
      startRepository(database, input.repositoryId);
      return { repository: knownRepository(stores, input.repositoryId) };
    },
  }),
  defineOperation({
    id: "repository_stop",
    method: "POST",
    path: "/api/repositories/{repositoryId}/stop",
    summary: "Stop Rerail in a repository's main checkout and the worktrees that follow it. Worktrees turned on or off themselves keep their state; rules and history are kept.",
    params: repositoryParameter,
    output: z.object({ repository: repositoryStartSchema }),
    handler: (stores, input) => {
      const repository = knownRepository(stores, input.repositoryId);
      if (repository.state === "never") return { repository };
      setCheckoutState(stores.database(), { repositoryId: input.repositoryId, root: repository.root, main: true }, "stopped");
      return { repository: knownRepository(stores, input.repositoryId) };
    },
  }),
  defineOperation({
    id: "rules_list",
    method: "GET",
    path: "/api/rules",
    summary: "List or search effective rules by kind, mode, enabled state, applicability, text, or conversation sample fields.",
    query: ruleFilterSchema,
    output: z.object({
      rules: z.array(listedRuleSchema),
      quarantined: z.array(quarantinedRuleSchema)
        .describe("Stored rows of the requested kind that fail validation and are skipped by every read and evaluation."),
    }),
    handler: (stores, input) => {
      const catalog = stores.rules();
      const packs = loadRulePacks();
      return {
        rules: catalog.list(ruleQuery(input)).map(stored => ({ ...stored, packId: rulePackIdOf(stored.rule.id, packs) })),
        quarantined: catalog.quarantined().filter(entry => input.kind === undefined || entry.kind === input.kind),
      };
    },
  }),
  // Must stay ahead of `/api/rules/{id}`: the router matches in registration order, so the
  // parameterized route would otherwise claim `activity` as a rule id.
  defineOperation({
    id: "rules_activity",
    method: "GET",
    path: "/api/rules/activity",
    summary: "Report what each rule actually caught since a given time: distinct violations, sessions, last sighting, how many audits evaluated it, and how many steering deliveries it drove. Use it to find rules worth disabling or rules that never fire.",
    query: z.object({ since: sinceParameter }).strict(),
    output: z.object({ since: z.number().int().nonnegative(), rules: z.record(z.string(), ruleActivitySchema) }),
    handler: (stores, input) => {
      const activity = stores.executions.ruleActivity(input.since);
      const rerails = stores.reader.steeringDeliveriesByRule(input.since);
      const rules: Record<string, z.infer<typeof ruleActivitySchema>> = {};
      for (const stored of stores.rules().list({})) {
        // Every catalog rule is reported, so a caller never has to infer absence from a missing key.
        const totals = stored.kind === "conversation" ? undefined : activity.get(stored.rule.id);
        rules[stored.rule.id] = {
          measured: stored.kind !== "conversation",
          distinctViolations: totals?.distinctViolations ?? 0,
          sessions: totals?.sessions ?? 0,
          lastSeenAt: totals?.lastSeenAt ?? null,
          audits: totals?.audits ?? 0,
          rerails: stored.kind === "conversation" ? 0 : rerails.get(stored.rule.id) ?? 0,
        };
      }
      return { since: input.since, rules };
    },
  }),
  defineOperation({
    id: "rules_get",
    method: "GET",
    path: "/api/rules/{id}",
    summary: "Fetch one rule by id.",
    params: ruleId,
    output: z.object({ rule: storedRuleSchema }),
    handler: (stores, input) => {
      const rule = stores.rules().get(input.id);
      if (rule === undefined) throw new ApiError(404, "rule-not-found", `rule ${input.id} does not exist`);
      return { rule };
    },
  }),
  defineOperation({
    id: "rules_upsert",
    method: "PUT",
    path: "/api/rules/{id}",
    summary: "Create or replace a rule. The body is validated against its kind's schema, and a file rule is compiled in the kernel, before it is stored.",
    params: ruleId,
    body: ruleUpsertSchema,
    output: z.object({ rule: storedRuleSchema }),
    handler: (stores, input) => {
      if (input.rule.id !== input.id) {
        throw new ApiError(400, "rule-id-mismatch", `path id ${input.id} does not match rule.id ${input.rule.id}`);
      }
      const options = {
        ...(input.enabled === undefined ? {} : { enabled: input.enabled }),
        ...(input.applicability === undefined ? {} : { applicability: input.applicability }),
        ...(input.revision === undefined ? {} : { revision: input.revision }),
      };
      const store = stores.rules();
      try {
        return { rule: input.kind === "file" ? store.upsertPolicy(input.rule, options) : store.upsertCosine(input.rule, options) };
      } catch (error) {
        if (error instanceof PolicyCompileError) throw new ApiError(400, "invalid-rule", error.message);
        if (error instanceof PolicyKernelUnavailableError) throw new ApiError(503, "kernel-unavailable", error.message);
        throw new ApiError(409, "rule-conflict", (error as Error).message);
      }
    },
  }),
  defineOperation({
    id: "rules_set_enabled",
    method: "PATCH",
    path: "/api/rules",
    summary: "Enable or disable rules in bulk, either by id or every rule matching a filter. Returns how many rows changed state.",
    body: z
      .object({
        enabled: z.boolean(),
        ids: z.array(z.string().min(1)).min(1).optional(),
        filter: ruleFilterSchema.optional(),
      })
      .strict()
      .superRefine((value, context) => {
        if ((value.ids === undefined) === (value.filter === undefined)) {
          context.addIssue({ code: "custom", path: ["ids"], message: "Provide exactly one of ids or filter." });
        }
      }),
    output: z.object({ changed: z.number().int().nonnegative() }),
    handler: (stores, input) => {
      try {
        return {
          changed: stores.rules().setEnabled(
            input.ids === undefined ? { filter: ruleQuery(input.filter ?? {}) } : { ids: input.ids },
            input.enabled,
          ),
        };
      } catch (error) {
        if (error instanceof QuarantinedGlobalRuleError) throw new ApiError(409, "rule-conflict", error.message);
        throw error;
      }
    },
  }),
  defineOperation({
    id: "rules_remove",
    method: "DELETE",
    path: "/api/rules/{id}",
    summary: "Delete one rule by id.",
    params: ruleId,
    query: z.object({ applicability: z.enum(RULE_APPLICABILITIES).optional() }).strict(),
    output: z.object({ removed: z.literal(1) }),
    handler: (stores, input) => {
      if (stores.rules().remove([input.id], input.applicability) === 0) {
        throw new ApiError(404, "rule-not-found", `rule ${input.id} does not exist`);
      }
      return { removed: 1 as const };
    },
  }),
  defineOperation({
    id: "rule_packs_list",
    method: "GET",
    path: "/api/rule-packs",
    summary: "List every rule pack that comes with Rerail: what it covers and its rules, each with a title, a group and the mode it starts in. Read-only; packs are not attached to repositories yet.",
    output: rulePackListSchema,
    handler: () => ({ packs: loadRulePacks().map(({ match: _match, rules, ...pack }) => ({ ...pack, rules: [...rules] })) }),
  }),
  defineOperation({
    id: "rule_packs_recommend",
    method: "GET",
    path: "/api/rule-packs/recommendations",
    summary: "Recommend rule packs for this repository by matching its files and package.json dependencies against each pack. Local; changes nothing. The daemon rescans a repository at most once a minute.",
    output: rulePackRecommendationsSchema,
    handler: stores => {
      const { repositoryId, workspaceRoot } = stores.rules().context();
      const recommendations = cachedRecommendRulePacks(workspaceRoot).map(({ pack, ruleCount, reason }) => ({ packId: pack.id, ruleCount, reason }));
      return { repositoryId, recommendations };
    },
  }),
  defineOperation({
    id: "audit_settings_get",
    method: "GET",
    path: "/api/sessions/{sessionId}/audit-settings",
    summary: "Fetch ongoing audit defaults and the first repository audit for one session's canonical workspace.",
    params: sessionId,
    output: policyControlsResponseSchema,
    handler: (stores, input) => {
      requireSession(stores, input.sessionId);
      return policyControlsResponse(stores, input.sessionId);
    },
  }),
  defineOperation({
    id: "audit_settings_update",
    method: "PATCH",
    path: "/api/sessions/{sessionId}/audit-settings",
    summary: "Update ongoing audit defaults for one session's canonical workspace.",
    params: sessionId,
    body: policySettingsPatchSchema,
    output: policyControlsResponseSchema,
    handler: (stores, input) => {
      requireSession(stores, input.sessionId);
      const { sessionId, ...patch } = input;
      stores.scanControls().updateSettings(sessionId, patch);
      return policyControlsResponse(stores, sessionId);
    },
  }),
  defineOperation({
    id: "audit_requests_create",
    method: "POST",
    path: "/api/sessions/{sessionId}/audit-requests",
    summary: "Queue an explicit audit for one session. Touched-file audits require paths.",
    params: sessionId,
    body: scanInputSchema,
    output: scanRequestSchema,
    handler: (stores, input) => {
      requireSession(stores, input.sessionId);
      const { sessionId, ...scan } = input;
      return stores.scanControls().requestScan(sessionId, scan);
    },
  }),
  defineOperation({
    id: "audit_requests_list",
    method: "GET",
    path: "/api/sessions/{sessionId}/audit-requests",
    summary: "List explicit audit requests and the ids of the audits each one produced for one session.",
    params: sessionId,
    output: z.object({ auditRequests: z.array(scanRequestSchema) }),
    handler: (stores, input) => {
      requireSession(stores, input.sessionId);
      return { auditRequests: stores.scanControls().listScans(input.sessionId) };
    },
  }),
  defineOperation({
    id: "repository_audits_list",
    method: "GET",
    path: "/api/repository-audits",
    summary: "List one full repository audit baseline per historically scanned repository.",
    query: repositoryDiagnosticQuerySchema,
    output: z.object({
      repositories: z.array(repositoryDiagnosticSummarySchema),
      total: z.number().int().nonnegative(),
    }),
    handler: (stores, input) => stores.executions.listRepositoryDiagnostics(repositoryDiagnosticQuery(input)),
  }),
  defineOperation({
    id: "repository_audit_entries_list",
    method: "GET",
    path: "/api/repository-audits/{auditId}/entries",
    summary: "Query paginated violations and failures from one repository audit baseline.",
    params: z.object({ auditId: z.string().min(1) }),
    query: repositoryDiagnosticEntryQuerySchema,
    output: z.object({
      entries: z.array(repositoryDiagnosticEntrySchema),
      total: z.number().int().nonnegative(),
    }),
    handler: (stores, input) => {
      const page = stores.executions.listRepositoryDiagnosticEntries(repositoryDiagnosticEntryQuery(input));
      if (page === undefined) throw new ApiError(404, "repository-audit-not-found", `repository audit ${input.auditId} does not exist`);
      return {
        entries: page.entries.map(entry => entry.kind === "violation"
          ? { kind: "violation" as const, violation: executablePolicyFindingSchema.parse(entry.payload) }
          : { kind: "failure" as const, failure: policyExecutionFailureSchema.parse(entry.payload) }),
        total: page.total,
      };
    },
  }),
  defineOperation({
    id: "audits_list",
    method: "GET",
    path: "/api/audits",
    summary: "List durable audit history, filtered independently by session, status, scope, or mode.",
    query: policyRunQuerySchema,
    output: z.object({ audits: z.array(policyRunRecordSchema), total: z.number().int().nonnegative() }),
    handler: (stores, input) => stores.executions.list(policyRunQuery(input)),
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
      ruleId: z.string().min(1).optional().describe("Restrict to one catalog rule id."),
    }).strict(),
    output: z.object({ total: z.number().int().nonnegative(), violations: z.array(findingRecordSchema) }),
    handler: (stores, input) => {
      const page = stores.executions.listFindingHistory({
        since: input.since,
        ...(input.limit === undefined ? {} : { limit: input.limit }),
        ...(input.offset === undefined ? {} : { offset: input.offset }),
        ...(input.ruleId === undefined ? {} : { ruleId: input.ruleId }),
      });
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
    query: z.object({ since: sinceParameter.optional().describe("Epoch milliseconds; only sessions started at or after this instant.") }).strict(),
    output: z.object({ sessions: z.array(sessionSummarySchema) }),
    handler: (stores, input) => ({
      sessions: [...(input.since === undefined ? stores.reader.sessions() : stores.reader.sessionsSince(input.since))],
    }),
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
    summary: `Page through one session's transcript events in sequence order, at most ${TRANSCRIPT_EVENT_PAGE_LIMIT} per page.`,
    params: sessionId,
    query: z.object({
      after: countParameter.optional().describe("Return events with a sequence strictly greater than this value."),
      limit: countParameter.optional(),
    }),
    output: z.object({
      events: z.array(eventRecordSchema),
      nextAfter: z.number().int().nonnegative(),
      pageLimit: z.literal(TRANSCRIPT_EVENT_PAGE_LIMIT),
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
        pageLimit: TRANSCRIPT_EVENT_PAGE_LIMIT as typeof TRANSCRIPT_EVENT_PAGE_LIMIT,
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
      const store = stores.transcripts();
      store.linkWork({
        sessionId: input.sessionId,
        kind: input.kind,
        reference: input.reference,
        ...(input.url === undefined || input.url === null ? {} : { url: input.url }),
      });
      return { workLinks: [...store.workLinks(input.sessionId)] };
    },
  }),
];

/** The flat argument object an MCP tool takes: path params, query, and body merged. */
export function operationInputShape(operation: Operation): z.ZodRawShape {
  return { ...operation.params?.shape, ...operation.query?.shape, ...operation.body?.shape };
}
