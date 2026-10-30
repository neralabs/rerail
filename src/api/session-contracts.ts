import { createSelectSchema } from "drizzle-zod";
import { z } from "zod";
import { audits, PRESENTATION_PRESENCES, sessions } from "../engine/storage/schema.js";

/** Shared by HTTP/OpenAPI and passive native readers, without importing server handlers. */
export const sessionSummarySchema = createSelectSchema(sessions)
  .extend({ eventCount: z.number().int().nonnegative() });
export const sessionResponseSchema = z.object({
  session: sessionSummarySchema,
  hud: z.object({
    activeRuleCount: z.number().int().nonnegative().nullable(),
    steeringCount: z.number().int().nonnegative().nullable(),
    // Optional: a footer reads a daemon of another installed version, which may not report it.
    awaitingApproval: z.number().int().nonnegative().nullable().optional()
      .describe("How many of the session repository's rules have content waiting for a person's approval; null when unknown."),
  }),
  audit: createSelectSchema(audits)
    .pick({ workspaceRoot: true, scope: true, mode: true, status: true })
    .extend({
      violationCount: z.number().int().nonnegative(),
      failureCount: z.number().int().nonnegative(),
      unlistedFailureCount: z.number().int().nonnegative(),
      blindFileCount: z.number().int().nonnegative(),
      gapCount: z.number().int().nonnegative(),
      outOfWorkRuleCount: z.number().int().nonnegative(),
      fileWorkStoppedRuleCount: z.number().int().nonnegative(),
    }).nullable(),
});
export const presentationDaemonSchema = z.object({
  pid: z.number().int().positive(),
  database: z.object({ path: z.string(), exists: z.boolean() }),
  native: z.object({ ok: z.boolean() }),
  workspaceRoot: z.string().optional(),
});
/** Which repository (and checkout) the dashboard opens on. */
export const rulesDashboardScopeSchema = z.object({ repositoryKey: z.string(), checkoutPath: z.string().nullable() });
export type RulesDashboardScope = z.infer<typeof rulesDashboardScopeSchema>;
export const presentationResponseSchema = z.object({
  presence: z.enum(PRESENTATION_PRESENCES),
  repositoryState: z.enum(["started", "stopped", "not-set-up"])
    .describe("Whether Rerail runs in the checkout: stopped after /rerail-stop or before its first start; not-set-up when started without a repository configuration."),
  scope: rulesDashboardScopeSchema.nullable().optional()
    .describe("The repository and checkout the workspace is in, for dashboard links; null when Rerail has not seen it."),
  context: z.object({ daemon: presentationDaemonSchema, session: sessionResponseSchema.nullable() }).optional(),
});

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
