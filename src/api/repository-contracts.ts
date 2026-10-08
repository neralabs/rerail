/**
 * Schemas of the repository-owned rule operations: targets, repositories and checkouts, effective rules,
 * settings, attachments, the library and By Rerail rules. Shared by the HTTP routes, the MCP tools and OpenAPI.
 */
import { isAbsolute } from "node:path";
import { z } from "zod";
import { codeRuleSchema } from "../code-rule-schema.js";
import { RULE_SETTING_OWNER_KINDS, RULE_SETTING_TARGET_KINDS, RULE_SOURCE_KINDS } from "../repository-model.js";
import { RULE_MODES } from "../rule-mode.js";
import { conversationRuleSchema } from "../rule-schema.js";
import { engineRegexIssues } from "../rule-regexes.js";
import { RULE_KINDS } from "../schema.js";

const START_STATES = ["started", "stopped", "not-set-up"] as const;

const repositoryField = z.string().min(1)
  .describe("Repository key (remote:<host>/<path> or clone:<clone id>) or one of its aliases.");
const checkoutField = z.string().min(1).refine(isAbsolute, "checkout must be an absolute path")
  .describe("Absolute path of a checkout, or of a folder inside one.");
const folderField = z.string()
  .describe("Folder relative to the repository root, POSIX; omitted or \"\" is the whole repository.");

/** Who an operation acts on. Resolved checkout first, then repository, then the calling agent's checkout. */
export const targetShape = {
  repository: repositoryField.optional(),
  checkout: checkoutField.optional(),
};
export const folderShape = { folder: folderField.optional() };

const startStateSchema = z.enum(START_STATES)
  .describe("stopped: Rerail is off; not-set-up: started without a configuration, so no rules run; started: rules run.");

export const repositorySummarySchema = z.object({
  repositoryKey: z.string(),
  name: z.string(),
  aliases: z.array(z.string()),
  setUp: z.boolean(),
  configuredAt: z.number().int().nullable(),
  started: z.boolean().describe("The repository's own on/off, which every checkout without its own value follows."),
  state: startStateSchema.describe("The repository's own state, before any checkout's own value."),
  checkoutCount: z.number().int().nonnegative(),
}).strict();

const checkoutStateSchema = z.object({
  checkoutPath: z.string(),
  started: z.boolean().nullable().describe("The checkout's own on/off; null follows the containing checkout, then the repository."),
  state: startStateSchema.describe("The effective state in this checkout."),
  startedBy: z.enum(["checkout", "parent", "repository"]),
  parentCheckoutPath: z.string().nullable().describe("The nearest recorded checkout of the same repository that contains this one."),
  onDisk: z.boolean(),
}).strict();

export const repositoryContextSchema = z.object({
  repository: repositorySummarySchema,
  clones: z.array(z.object({ cloneId: z.string(), checkouts: z.array(checkoutStateSchema) }).strict()),
  checkout: z.object({
    checkoutPath: z.string(),
    folder: z.string().describe("The folder of the target (or the calling agent's folder) inside the checkout."),
    state: startStateSchema,
  }).strict().nullable().describe("The checkout the target or the calling agent names; null for a repository-only target."),
}).strict();

export const deciderSchema = z.discriminatedUnion("by", [
  z.object({ by: z.literal("checkout"), checkoutPath: z.string(), folder: z.string(), target: z.enum(RULE_SETTING_TARGET_KINDS) }).strict(),
  z.object({ by: z.literal("repository"), folder: z.string(), target: z.enum(RULE_SETTING_TARGET_KINDS) }).strict(),
  z.object({ by: z.literal("pack-start"), packId: z.string() }).strict(),
  z.object({ by: z.literal("rule-default") }).strict(),
]);

const ruleSourceSchema = z.object({
  kind: z.enum(RULE_SOURCE_KINDS),
  id: z.string(),
  packId: z.string().nullable(),
}).strict();

const effectiveRuleBase = {
  source: ruleSourceSchema.describe("Where the rule comes from; a rule in an added pack belongs to the pack."),
  folder: z.string().describe("Folder of the attachment that makes the rule present."),
  on: z.boolean(),
  mode: z.enum(RULE_MODES),
  onSource: deciderSchema.describe("What decided on/off: a checkout's or the repository's setting, the pack's starting value, or the rule's default."),
  modeSource: deciderSchema,
  revision: z.number().int().positive(),
  payloadHash: z.string(),
};
export const effectiveRuleSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("file"), rule: codeRuleSchema, ...effectiveRuleBase }).strict(),
  z.object({ kind: z.literal("conversation"), rule: conversationRuleSchema, ...effectiveRuleBase }).strict(),
]);

export const unavailableReferenceSchema = z.object({
  sourceKind: z.enum(RULE_SOURCE_KINDS),
  sourceId: z.string(),
  folder: z.string(),
  reason: z.enum(["missing", "conflicts-with-library"]),
}).strict();

export const repositoryRulesSchema = z.object({
  repositoryKey: z.string(),
  checkoutPath: z.string().nullable().describe("Checkout whose own settings (and its parents') are applied; null for the repository alone."),
  setUp: z.boolean(),
  rules: z.array(effectiveRuleSchema),
  unavailable: z.array(unavailableReferenceSchema),
}).strict();

const settingValues = {
  on: z.boolean().nullable().optional().describe("true/false sets on/off; null clears it; omitted keeps the stored value."),
  mode: z.enum(RULE_MODES).nullable().optional().describe("Sets the mode; null clears it; omitted keeps the stored value."),
};
const settingTargetSchema = z.object({ kind: z.enum(RULE_SETTING_TARGET_KINDS), id: z.string().min(1) }).strict();

export const settingsSetBodySchema = z.object({
  ...targetShape,
  ...folderShape,
  owner: z.enum(RULE_SETTING_OWNER_KINDS).describe("repository: the default for every checkout; checkout: this worktree only."),
  targets: z.array(settingTargetSchema).min(1),
  ...settingValues,
}).strict();

export const ruleSettingRecordSchema = z.object({
  owner: z.enum(RULE_SETTING_OWNER_KINDS),
  ownerId: z.string().describe("Repository key or checkout path."),
  folder: z.string(),
  targetKind: z.enum(RULE_SETTING_TARGET_KINDS),
  targetId: z.string(),
  on: z.boolean().nullable(),
  mode: z.enum(RULE_MODES).nullable(),
}).strict();

const attachmentInputSchema = z.object({
  sourceKind: z.enum(RULE_SOURCE_KINDS),
  sourceId: z.string().min(1),
  folder: folderField.optional(),
}).strict();

export const attachmentsSetBodySchema = z.object({
  ...targetShape,
  add: z.array(attachmentInputSchema).optional(),
  remove: z.array(attachmentInputSchema).optional(),
  settings: z.array(z.object({ ...folderShape, targetKind: z.enum(RULE_SETTING_TARGET_KINDS), targetId: z.string().min(1), ...settingValues }).strict())
    .optional().describe("Repository-owned settings written in the same transaction."),
  configure: z.boolean().optional()
    .describe("Mark the repository set up even when nothing is added (\"I'll pick rule packs myself\"). Adding anything sets it up too."),
}).strict();

export const attachmentRecordSchema = z.object({
  folder: z.string(),
  sourceKind: z.enum(RULE_SOURCE_KINDS),
  sourceId: z.string(),
}).strict();

const libraryRuleBase = {
  revision: z.number().int().positive(),
  payloadHash: z.string(),
  copiedFromRuleId: z.string().nullable().describe("The By Rerail rule this rule is an edited copy of."),
  createdAt: z.number().int(),
  updatedAt: z.number().int(),
};
export const libraryRuleRecordSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("file"), rule: codeRuleSchema, ...libraryRuleBase }).strict(),
  z.object({ kind: z.literal("conversation"), rule: conversationRuleSchema, ...libraryRuleBase }).strict(),
]);

export const quarantinedRuleSchema = z.object({
  ruleId: z.string(),
  kind: z.enum(RULE_KINDS),
  owner: z.string(),
  reason: z.string(),
}).strict();

export const libraryUpsertBodySchema = z.object({
  kind: z.enum(RULE_KINDS),
  rule: z.union([codeRuleSchema, conversationRuleSchema]),
  revision: z.number().int().positive().optional().describe("Conversation rules only; file rules carry rule.revision."),
  ...targetShape,
  ...folderShape,
  attach: z.boolean().optional().describe("Also add the rule to the target repository, in folder."),
}).strict().superRefine((value, context) => {
  if ((value.kind === "file") !== ("clauses" in value.rule)) {
    context.addIssue({ code: "custom", path: ["rule"], message: `rule body does not match kind ${value.kind}` });
  }
  if (value.kind === "file" && value.revision !== undefined) {
    context.addIssue({ code: "custom", path: ["revision"], message: "file rules take their revision from rule.revision" });
  }
  if (value.kind === "conversation" && !("clauses" in value.rule)) {
    for (const issue of engineRegexIssues(value.rule)) {
      context.addIssue({ code: "custom", path: ["rule", ...issue.path], message: issue.message });
    }
  }
});

export const libraryUpsertResultSchema = z.object({
  rule: libraryRuleRecordSchema,
  copiedFromRuleId: z.string().nullable().describe("Set when the write edited a By Rerail rule and stored a copy instead."),
  attached: z.object({ repositoryKey: z.string(), folders: z.array(z.string()) }).strict().nullable()
    .describe("Where the rule (or the copy) was added; a copy also switches the original off there."),
}).strict();

export const rerailRuleSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("file"), rule: codeRuleSchema, packId: z.string().nullable() }).strict(),
  z.object({ kind: z.literal("conversation"), rule: conversationRuleSchema, packId: z.string().nullable() }).strict(),
]);

export const startBodySchema = z.object({
  ...targetShape,
  scope: z.enum(["checkout", "repository"])
    .describe("checkout: this checkout and the checkouts inside it; repository: every checkout, clearing their own values."),
}).strict();

export const startResultSchema = z.object({
  repository: repositorySummarySchema,
  checkout: z.object({ checkoutPath: z.string(), state: startStateSchema }).strict().nullable(),
}).strict();

export const linkBodySchema = z.object({
  ...targetShape,
  address: z.string().min(1).describe("A remote address (https, ssh or scp form) that should resolve to this repository."),
}).strict();
