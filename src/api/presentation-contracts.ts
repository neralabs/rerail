import { z } from "zod";
import { REPOSITORY_START_STATES } from "../engine/catalog/repository-model.js";
import { PRESENTATION_PRESENCES } from "../engine/storage/presentation-presence.js";

/*
 * What a passive footer reader (the Claude Code status line, every host's footer) needs from `GET /api/presentation`,
 * in plain zod. The daemon's own contract (`session-contracts.ts`) derives the session and audit rows from the Drizzle
 * tables, which links Drizzle, the storage schema and every code-rule schema into whatever imports it; the status line
 * runs every few seconds per session and paid for loading them on each run. The parts both share live here, and a
 * daemon's answer under its contract parses here too (`test/api/presentation-contracts.test.ts`); fields the footer
 * does not read are kept as they come.
 */

export const presentationDaemonSchema = z.object({
  pid: z.number().int().positive(),
  database: z.object({ path: z.string(), exists: z.boolean() }),
  native: z.object({ ok: z.boolean() }),
  workspaceRoot: z.string().optional(),
});

/** Which repository (and checkout) the dashboard opens on. */
export const rulesDashboardScopeSchema = z.object({ repositoryKey: z.string(), checkoutPath: z.string().nullable() });
export type RulesDashboardScope = z.infer<typeof rulesDashboardScopeSchema>;

/** The answer's start state, as the daemon's contract describes it. */
export const repositoryStateSchema = z.enum(REPOSITORY_START_STATES)
  .describe("Whether Rerail runs in the checkout: stopped after /rerail-stop or before its first start; not-set-up when started without a repository configuration.");

/** The answer's scope, as the daemon's contract describes it. */
export const presentationScopeSchema = rulesDashboardScopeSchema.nullable().optional()
  .describe("The repository and checkout the workspace is in, for dashboard links; null when Rerail has not seen it.");

/** The session counters the footer shows. */
export const sessionHudSchema = z.object({
  activeRuleCount: z.number().int().nonnegative().nullable(),
  steeringCount: z.number().int().nonnegative().nullable(),
  // Optional: a footer reader may meet a daemon from before overlays, which sends none.
  overlays: z.array(z.string()).optional()
    .describe("Labels of the rule overlays in force in the session's checkout, e.g. worktree while its .rerail/rules holds rules."),
  awaitingApproval: z.number().int().nonnegative().nullable().optional()
    .describe("How many rules and packs that are on run in Log in the session's checkout, waiting for a person's approval; null when the rule count is not current."),
});

/** The latest audit's counts the footer shows. */
export const auditCountsSchema = z.object({
  violationCount: z.number().int().nonnegative(),
  failureCount: z.number().int().nonnegative(),
  unlistedFailureCount: z.number().int().nonnegative(),
  blindFileCount: z.number().int().nonnegative(),
  gapCount: z.number().int().nonnegative(),
  outOfWorkRuleCount: z.number().int().nonnegative(),
  fileWorkStoppedRuleCount: z.number().int().nonnegative(),
});

/** A recorded session as the footer reads it: who it is, its counters, and its latest audit. */
export const presentedSessionSchema = z.object({
  session: z.looseObject({ sessionId: z.string(), host: z.string(), cwd: z.string() }),
  hud: sessionHudSchema,
  audit: z.looseObject({ workspaceRoot: z.string(), status: z.string() }).extend(auditCountsSchema.shape).nullable(),
});

export const presentationReadSchema = z.object({
  presence: z.enum(PRESENTATION_PRESENCES),
  repositoryState: repositoryStateSchema,
  scope: presentationScopeSchema,
  context: z.object({ daemon: presentationDaemonSchema, session: presentedSessionSchema.nullable() }).optional(),
});
export type PresentationRead = z.infer<typeof presentationReadSchema>;
export type PresentedSession = z.infer<typeof presentedSessionSchema>;

/** `<origin>/?repository=<key>&checkout=<path>#rules`: the rules view of the dashboard, scoped when a scope is known. */
export function rulesDashboardUrl(origin: URL | string, scope?: RulesDashboardScope | null): string {
  const url = new URL(origin);
  url.search = "";
  if (scope !== undefined && scope !== null) {
    url.searchParams.set("repository", scope.repositoryKey);
    if (scope.checkoutPath !== null) url.searchParams.set("checkout", scope.checkoutPath);
  }
  url.hash = "rules";
  return url.href;
}
