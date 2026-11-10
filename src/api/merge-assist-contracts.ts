/**
 * Schemas of the Merge Assist operations: the checkout they act on, and what each change takes. Every change answers
 * with the checkout's {@link mergeAssistStatusSchema}; reading the settings answers {@link mergeAssistSettingsSchema}.
 * Shared by the HTTP routes, the MCP tools and OpenAPI.
 * Internal testing: Merge Assist is experimental (docs/merge-assist-v1-build.md).
 */
import { z } from "zod";
import { CANDIDATE_ID_PATTERN, CANDIDATE_STATES } from "../engine/merge-assist/candidate.js";
import {
  CONFLICT_ACTIONS,
  CONFLICT_DELIVERY_MODES,
  conflictHookCommandSchema,
  DEFAULT_HOOK_TIMEOUT_SECONDS,
  FINDING_ACTIONS,
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
    .describe("true turns Merge Assist on for the checkout's repository; false turns it off and removes every decision it applied."),
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

export const mergeAssistCandidateBodySchema = z.object({
  ...targetShape,
  candidateId: z.string().regex(CANDIDATE_ID_PATTERN, "a candidate id is candidate:<lowercase letters, digits and dashes>")
    .describe("One of this branch's own decisions, as merge_assist_get lists it under own."),
  state: z.enum(CANDIDATE_STATES)
    .describe("rejected: not a rule (Not a rule in the dashboard); active: restore it. A coding agent may only reject."),
}).strict();

export const mergeAssistFindingBodySchema = z.object({
  ...targetShape,
  findingId: z.string().min(1).describe("A finding's id, as merge_assist_get lists it under findings."),
  action: z.enum(FINDING_ACTIONS)
    .describe("adopt: apply a review finding; dismiss: hide a finding until its candidate changes; undo: remove an applied one. A coding agent may only dismiss."),
}).strict();

export const mergeAssistConflictBodySchema = z.object({
  ...targetShape,
  conflictId: z.string().min(1).describe("An in-flight conflict's id, as merge_assist_get lists it under conflicts."),
  action: z.enum(CONFLICT_ACTIONS)
    .describe("dismiss: not a conflict. Experimental verdicts (RERAIL_EXPERIMENTAL_CONFLICT_VERDICTS=1 where Merge Assist runs; refused with invalid-state otherwise): mine keeps this branch's side, theirs takes the other side's, new writes a new decision (text required). Only a person resolves a conflict."),
  text: z.string().min(1).max(600).optional()
    .describe("The decision's text: required for new; for mine and theirs it defaults to that side's summary."),
}).strict().refine(body => body.action !== "new" || body.text !== undefined, { message: "a new decision needs its text", path: ["text"] });
