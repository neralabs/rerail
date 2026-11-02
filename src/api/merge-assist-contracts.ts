/**
 * Schemas of the Merge Assist operations: the checkout they act on, and what each change takes. Every operation
 * answers with the checkout's {@link mergeAssistStatusSchema}. Shared by the HTTP routes, the MCP tools and OpenAPI.
 * Internal testing: Merge Assist is experimental (docs/merge-assist-prd.md).
 */
import { z } from "zod";
import { CANDIDATE_ID_PATTERN, CANDIDATE_STATES } from "../engine/merge-assist/candidate.js";
import { FINDING_ACTIONS, mergeAssistStatusSchema } from "../engine/merge-assist/model.js";
import { targetShape } from "./repository-contracts.js";

export { mergeAssistStatusSchema };

/** The checkout Merge Assist acts on: `checkout`, else the repository's main checkout, else the calling agent's. */
export const mergeAssistTargetSchema = z.object({ ...targetShape }).strict();

export const mergeAssistSettingsBodySchema = z.object({
  ...targetShape,
  enabled: z.boolean()
    .describe("true turns Merge Assist on for the checkout's repository; false turns it off and removes every decision it applied."),
}).strict();

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
