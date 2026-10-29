import { z } from "zod";
import { RULE_MODES } from "../engine/catalog/rule-mode.js";
import { RULE_PACK_CATEGORIES } from "../engine/catalog/rule-packs.js";
import { RULE_KINDS } from "../engine/storage/schema.js";

const ruleExampleSchema = z.object({ path: z.string().optional(), text: z.string() }).strict();

export const rulePackSchema = z.object({
  id: z.string(),
  name: z.string(),
  version: z.string().nullable().describe("The installed version of the pack's npm package."),
  packageName: z.string().nullable()
    .describe("The pack's npm package: @neralabs/rerail-pack-<id> for Rerail's own, else a third party's rerail-pack-<id> or @<scope>/rerail-pack-<id>; null for a pack directory without one."),
  publisher: z.string()
    .describe("Who publishes the pack: rerail for Rerail's own; else the npm scope (@acme), or the package name when it has no scope. Rerail has not reviewed a third party's pack."),
  category: z.enum(RULE_PACK_CATEGORIES),
  covers: z.string(),
  reason: z.string().describe("The pack's own summary of what makes it fit; the reason for a repository comes from the recommendations."),
  honourSuppressionComments: z.boolean().describe("Whether the pack's rules honour their linters' suppression comments (# noqa, eslint-disable)."),
  rules: z.array(z.object({
    id: z.string(),
    kind: z.enum(RULE_KINDS).describe("A code rule checks source files; a conversation rule watches the coding agent's tool calls."),
    title: z.string(),
    group: z.string().describe("The group the rule is filed under inside its pack, e.g. Complexity or api."),
    mode: z.enum(RULE_MODES).describe("The rule mode the pack starts it in."),
    on: z.boolean().describe("Whether the pack starts the rule on."),
    examples: z.object({ caught: ruleExampleSchema, passes: ruleExampleSchema }).strict().optional()
      .describe("One example the rule catches and one it lets pass, where the pack has them."),
  }).strict()),
}).strict();

export const skippedRulePackSchema = z.object({
  path: z.string().describe("The installed pack's directory."),
  packageName: z.string().nullable(),
  version: z.string().nullable(),
  reason: z.string().describe("Why the pack was left out, e.g. a core range that excludes this Rerail, or a rule the engine rejects."),
}).strict();

export const rulePackListSchema = z.object({
  packs: z.array(rulePackSchema),
  skipped: z.array(skippedRulePackSchema).describe("Installed packs left out because they failed a check; the others load."),
});

export const rulePackRecommendationsSchema = z.object({
  repositoryKey: z.string(),
  checkoutPath: z.string().describe("The checkout that was scanned: the target's, else the main checkout on disk, else the most recently seen."),
  recommendations: z.array(z.object({
    packId: z.string(),
    ruleCount: z.number().int().positive(),
    reason: z.string(),
    folder: z.string().describe("Folder to add the pack in, relative to the repository root; \"\" is the whole repository."),
  }).strict()),
});
