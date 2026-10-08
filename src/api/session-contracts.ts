import { createSelectSchema } from "drizzle-zod";
import { z } from "zod";
import { audits, PRESENTATION_PRESENCES, sessions } from "../schema.js";

/** Shared by HTTP/OpenAPI and passive native readers, without importing server handlers. */
export const sessionSummarySchema = createSelectSchema(sessions)
  .omit({ startGated: true })
  .extend({ eventCount: z.number().int().nonnegative() });
export const sessionResponseSchema = z.object({
  session: sessionSummarySchema,
  hud: z.object({
    activeRuleCount: z.number().int().nonnegative().nullable(),
    steeringCount: z.number().int().nonnegative().nullable(),
  }),
  audit: createSelectSchema(audits)
    .pick({ workspaceRoot: true, scope: true, mode: true, status: true })
    .extend({ violationCount: z.number().int().nonnegative(), failureCount: z.number().int().nonnegative() }).nullable(),
});
export const presentationDaemonSchema = z.object({
  pid: z.number().int().positive(),
  database: z.object({ path: z.string(), exists: z.boolean() }),
  native: z.object({ ok: z.boolean() }),
  workspaceRoot: z.string().optional(),
  repositoryId: z.string().optional(),
});
export const presentationResponseSchema = z.object({
  presence: z.enum(PRESENTATION_PRESENCES),
  repositoryState: z.enum(["started", "stopped", "never"])
    .describe("Whether Rerail is started in the repository: stopped after /rerail-stop, never before its first start."),
  context: z.object({ daemon: presentationDaemonSchema, session: sessionResponseSchema.nullable() }).optional(),
});
