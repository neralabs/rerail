/**
 * Schemas of the Merge Assist operations: the checkout they act on, and what each change takes. Every change answers
 * with the checkout's {@link mergeAssistStatusSchema}; reading the settings answers {@link mergeAssistSettingsSchema}.
 * Shared by the HTTP routes, the MCP tools and OpenAPI.
 * Internal testing: Merge Assist is experimental (docs/merge-assist-v1-build.md).
 */
import { z } from "zod";
import {
  CONFLICT_DELIVERY_MODES,
  conflictHookCommandSchema,
  DEFAULT_HOOK_TIMEOUT_SECONDS,
  mergeAssistSettingsSchema,
  mergeAssistStatusSchema,
} from "../engine/merge-assist/model.js";
import { targetShape } from "./repository-contracts.js";

export { mergeAssistSettingsSchema, mergeAssistStatusSchema };

/** The checkout Merge Assist acts on: `checkout`, else the repository's main checkout, else the calling agent's. */
export const mergeAssistTargetSchema = z.object({ ...targetShape }).strict();

export const mergeAssistSettingsBodySchema = z.object({
  ...targetShape,
  enabled: z.boolean().optional()
    .describe("true turns Merge Assist on for the checkout's repository; false turns it off and clears the conflicts it found, dismissals included."),
  delivery: z.enum(CONFLICT_DELIVERY_MODES).optional()
    .describe("Where conflicts reach the person: chat (a note through the agent), terminal (the status line and the person's line), or both."),
  hook: conflictHookCommandSchema.extend({
    timeoutSeconds: conflictHookCommandSchema.shape.timeoutSeconds.optional()
      .describe(`Seconds before the command is stopped: ${DEFAULT_HOOK_TIMEOUT_SECONDS} when left out.`),
  }).strict().nullable().optional()
    .describe("The conflict hook command, an argv run without a shell on every conflict event; null removes it."),
}).strict().refine(
  body => body.enabled !== undefined || body.delivery !== undefined || body.hook !== undefined,
  { message: "name at least one of enabled, delivery and hook" },
);

export const mergeAssistDismissBodySchema = z.object({
  ...targetShape,
  conflictId: z.string().min(1).describe("An in-flight conflict's id, as merge_assist_get lists it under conflicts."),
}).strict();
