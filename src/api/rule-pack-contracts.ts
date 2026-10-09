import { z } from "zod";
import { RULE_MODES } from "../engine/catalog/rule-mode.js";
import { RULE_PACK_CATEGORIES } from "../engine/catalog/rule-packs.js";
import { RULE_KINDS } from "../engine/storage/schema.js";

const ruleExampleSchema = z.object({ path: z.string().optional(), text: z.string() }).strict();

export const rulePackSchema = z.object({
  id: z.string(),
  name: z.string(),
  category: z.enum(RULE_PACK_CATEGORIES),
  covers: z.string(),
  reason: z.string().describe("The pack's own summary of what makes it fit; the reason for a repository comes from the recommendations."),
  honourSuppressionComments: z.boolean().describe("Whether the pack's rules honour their linters' suppression comments (# noqa, eslint-disable)."),
  rules: z.array(z.object({
    id: z.string(),
    kind: z.enum(RULE_KINDS).describe("A file rule checks code; a conversation rule watches the coding agent's tool calls."),
    title: z.string(),
    group: z.string().describe("The group the rule is filed under inside its pack, e.g. TSLint or api."),
    mode: z.enum(RULE_MODES).describe("The rule mode the pack starts it in."),
    on: z.boolean().describe("Whether the pack starts the rule on."),
    examples: z.object({ caught: ruleExampleSchema, passes: ruleExampleSchema }).strict().optional()
      .describe("One example the rule catches and one it lets pass, where the pack has them."),
  }).strict()),
}).strict();

export const rulePackListSchema = z.object({ packs: z.array(rulePackSchema) });

export const rulePackRecommendationsSchema = z.object({
  repositoryId: z.string(),
  recommendations: z.array(z.object({ packId: z.string(), ruleCount: z.number().int().positive(), reason: z.string() }).strict()),
});
