import { createSelectSchema } from "drizzle-zod";
import { z } from "zod";
import { audits, PRESENTATION_PRESENCES, sessions } from "../engine/storage/schema.js";
import {
  auditCountsSchema,
  presentationDaemonSchema,
  presentationScopeSchema,
  repositoryStateSchema,
  rulesDashboardScopeSchema,
  rulesDashboardUrl,
  sessionHudSchema,
  type RulesDashboardScope,
} from "./presentation-contracts.js";

export { presentationDaemonSchema, rulesDashboardScopeSchema, rulesDashboardUrl, type RulesDashboardScope };

/** Shared by HTTP/OpenAPI and passive native readers, without importing server handlers. */
export const sessionSummarySchema = createSelectSchema(sessions)
  .extend({ eventCount: z.number().int().nonnegative() });
export const sessionResponseSchema = z.object({
  session: sessionSummarySchema,
  hud: sessionHudSchema,
  audit: createSelectSchema(audits)
    .pick({ workspaceRoot: true, scope: true, mode: true, status: true })
    .extend(auditCountsSchema.shape).nullable(),
});
export const presentationResponseSchema = z.object({
  presence: z.enum(PRESENTATION_PRESENCES),
  repositoryState: repositoryStateSchema,
  scope: presentationScopeSchema,
  context: z.object({ daemon: presentationDaemonSchema, session: sessionResponseSchema.nullable() }).optional(),
});
