/**
 * Schemas of the repository-owned rule operations: targets, repositories and checkouts, effective rules,
 * settings, attachments, the library and By Rerail rules. Shared by the HTTP routes, the MCP tools and OpenAPI.
 */
import { isAbsolute } from "node:path";
import { z } from "zod";
import { LANGUAGE_NOT_SUPPORTED, codeRuleLanguageNotSupported, type CodeRuleLanguageNotSupported } from "../engine/code-rule/code-rule-language-support.js";
import { codeRuleSchema, codeRuleValueSchema } from "../engine/code-rule/code-rule-schema.js";
import { RULE_SETTING_OWNER_KINDS, RULE_SETTING_TARGET_KINDS, RULE_SOURCE_KINDS, RULE_SUPPRESSIONS } from "../engine/catalog/repository-model.js";
import { RULE_MODES } from "../engine/catalog/rule-mode.js";
import { conversationRuleSchema } from "../engine/catalog/rule-schema.js";
import { engineRegexIssues } from "../engine/catalog/rule-regexes.js";
import { RULE_APPROVAL_OWNERS, RULE_KINDS } from "../engine/storage/schema.js";

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
  followGitignore: z.boolean()
    .describe("Whether audits leave out files Git ignores (.gitignore, .git/info/exclude, global excludes); sessions started after a change use the new value."),
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

export const inactiveField = {
  inactive: z
    .object({ code: z.literal(LANGUAGE_NOT_SUPPORTED), reason: z.string() })
    .strict()
    .optional()
    .describe("Present when the rule is stored but never evaluated: its language, or a capability it reads, is not supported yet."),
};

/** The `inactive` field for a code rule whose language, or a capability it reads, the engine does not read yet. */
export function inactiveOf(kind: string, rule: unknown): { inactive?: CodeRuleLanguageNotSupported } {
  const unsupported = kind === "code" ? codeRuleLanguageNotSupported(rule as Parameters<typeof codeRuleLanguageNotSupported>[0]) : undefined;
  return unsupported === undefined ? {} : { inactive: unsupported };
}

const ruleSourceSchema = z.object({
  kind: z.enum(RULE_SOURCE_KINDS),
  id: z.string(),
  packId: z.string().nullable(),
}).strict();

const ruleConfigSchema = z.record(z.string(), codeRuleValueSchema);

export const ruleApprovalSchema = z.object({
  state: z.enum(["shipped", "approved", "pending"])
    .describe("shipped: Nera shipped the rule (a pack installed from the npm registry), so it needs no approval. approved: a person approved this content. pending: newer content waits for a person's approval; the content approved at approvedAt runs meanwhile, or, with approvedAt null, the rule runs nowhere yet."),
  approvedAt: z.number().int().nullable().describe("Epoch milliseconds the content that runs was approved; null when none was, or none is needed."),
}).strict();

const contentHashField = z.string().min(1)
  .describe("The rule content's digest (sha256 of everything but mode and revision); an approval binds to it.");

const effectiveRuleBase = {
  source: ruleSourceSchema.describe("Where the rule comes from; a rule in an added pack belongs to the pack."),
  folder: z.string().describe("Folder of the attachment that makes the rule present."),
  on: z.boolean(),
  mode: z.enum(RULE_MODES),
  onSource: deciderSchema.describe("What decided on/off: a checkout's or the repository's setting, the pack's starting value, or the rule's default."),
  modeSource: deciderSchema,
  suppressions: z.enum(RULE_SUPPRESSIONS)
    .describe("honour: a linter's suppression comment (eslint-disable, noqa, ...) silences the rule where its document reads that linter's comments; ignore: no linter's comment silences it. Rerail's own rerail-ignore comment silences the rule either way."),
  suppressionsSource: deciderSchema.describe("What decided suppressions; a pack starts from its honourSuppressionComments."),
  config: ruleConfigSchema.nullable().describe("Option values the settings give this rule by Rerail, merged per option; null when none is set."),
  configSource: z.record(z.string(), deciderSchema).describe("The setting that decided each option in config."),
  configError: z.string().nullable().describe("Why config does not apply to the rule's document; the rule then runs nowhere."),
  revision: z.number().int().positive(),
  payloadHash: z.string(),
  approval: ruleApprovalSchema.describe("Whose content runs: Nera's, or what a person approved; pending: an edit waits for approval and the approved content runs meanwhile."),
  addedAt: z.number().int().nullable()
    .describe("Epoch milliseconds the attachment that brings the rule in (its pack, or the rule itself) was added at this folder."),
  updatedAt: z.number().int().nullable().describe("Epoch milliseconds a rule created by you was last edited; null for rules by Rerail."),
};
export const effectiveRuleSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("code"), rule: codeRuleSchema, ...effectiveRuleBase, ...inactiveField }).strict(),
  z.object({ kind: z.literal("conversation"), rule: conversationRuleSchema, ...effectiveRuleBase }).strict(),
]);

export const unavailableReferenceSchema = z.object({
  sourceKind: z.enum(RULE_SOURCE_KINDS),
  sourceId: z.string(),
  folder: z.string(),
  reason: z.enum(["missing", "conflicts-with-library", "awaiting-approval"])
    .describe("awaiting-approval: the rule has no approved content yet, so it runs nowhere until a person approves it."),
}).strict();

export const repositoryRulesSchema = z.object({
  repositoryKey: z.string(),
  checkoutPath: z.string().nullable().describe("Checkout whose own settings (and its parents') are applied; null for the repository alone."),
  setUp: z.boolean(),
  rules: z.array(effectiveRuleSchema),
  unavailable: z.array(unavailableReferenceSchema),
  awaitingApproval: z.number().int().nonnegative()
    .describe("How many of the repository's rules have content waiting for a person's approval: new ones that run nowhere yet, and edited ones that run their approved content meanwhile."),
}).strict();

const settingValues = {
  on: z.boolean().nullable().optional().describe("true/false sets on/off; null clears it; omitted keeps the stored value."),
  mode: z.enum(RULE_MODES).nullable().optional().describe("Sets the mode; null clears it; omitted keeps the stored value. block is in internal testing and is delivered like steer."),
  suppressions: z.enum(RULE_SUPPRESSIONS).nullable().optional()
    .describe("honour or ignore linters' suppression comments (eslint-disable, noqa, ...) for the rule; null clears it; omitted keeps the stored value."),
  config: ruleConfigSchema.nullable().optional()
    .describe("Option values of one code rule by Rerail (option name → value), replacing the stored ones; rule targets over the whole repository (folder \"\") only. null clears it; omitted keeps the stored value."),
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
  suppressions: z.enum(RULE_SUPPRESSIONS).nullable(),
  config: ruleConfigSchema.nullable(),
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
  contentHash: contentHashField,
  approval: ruleApprovalSchema.describe("approved: this content runs where the rule is added. pending: it waits for a person's approval."),
  copiedFromRuleId: z.string().nullable().describe("The By Rerail rule this rule is an edited copy of."),
  createdAt: z.number().int(),
  updatedAt: z.number().int(),
};
export const libraryRuleRecordSchema = z.discriminatedUnion("kind", [
  z.object({
    kind: z.literal("code"),
    rule: codeRuleSchema,
    approvedRule: codeRuleSchema.nullable().describe("While this content is pending: the approved content, which runs meanwhile; else null."),
    ...libraryRuleBase,
    ...inactiveField,
  }).strict(),
  z.object({
    kind: z.literal("conversation"),
    rule: conversationRuleSchema,
    approvedRule: conversationRuleSchema.nullable().describe("While this content is pending: the approved content, which runs meanwhile; else null."),
    ...libraryRuleBase,
  }).strict(),
]);

export const quarantinedRuleSchema = z.object({
  ruleId: z.string(),
  kind: z.enum(RULE_KINDS),
  owner: z.string(),
  reason: z.string(),
  code: z.literal(LANGUAGE_NOT_SUPPORTED).optional(),
}).strict();

export const libraryUpsertBodySchema = z.object({
  kind: z.enum(RULE_KINDS).describe("code, or conversation, which is in internal testing."),
  rule: z.union([codeRuleSchema, conversationRuleSchema]),
  revision: z.number().int().positive().optional().describe("Conversation rules only; code rules carry rule.revision."),
  ...targetShape,
  ...folderShape,
  attach: z.boolean().optional().describe("Also add the rule to the target repository, in folder."),
}).strict().superRefine((value, context) => {
  if ((value.kind === "code") !== ("clauses" in value.rule)) {
    context.addIssue({ code: "custom", path: ["rule"], message: `rule body does not match kind ${value.kind}` });
  }
  if (value.kind === "code" && value.revision !== undefined) {
    context.addIssue({ code: "custom", path: ["revision"], message: "code rules take their revision from rule.revision" });
  }
  if (value.kind === "conversation" && !("clauses" in value.rule)) {
    for (const issue of engineRegexIssues(value.rule)) {
      context.addIssue({ code: "custom", path: ["rule", ...issue.path], message: issue.message });
    }
  }
});

/** The note a write that puts a rule on `block` answers with: block does not block in this release, and steers. */
export const blockNoteShape = {
  note: z.string().optional().describe("Set when the write puts a rule on block: block does not block in this release, and the rule steers instead."),
};

export const libraryUpsertResultSchema = z.object({
  rule: libraryRuleRecordSchema,
  copiedFromRuleId: z.string().nullable().describe("Set when the write edited a By Rerail rule and stored a copy instead."),
  attached: z.object({ repositoryKey: z.string(), folders: z.array(z.string()) }).strict().nullable()
    .describe("Where the rule (or the copy) was added; a copy switches the original off there once it is approved."),
  ...blockNoteShape,
}).strict();

export const rerailRuleSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("code"), rule: codeRuleSchema, packId: z.string().nullable(), approval: ruleApprovalSchema, ...inactiveField }).strict(),
  z.object({ kind: z.literal("conversation"), rule: conversationRuleSchema, packId: z.string().nullable(), approval: ruleApprovalSchema }).strict(),
]);

const approvedContentSchema = <T extends z.ZodType>(rule: T) => z.object({ rule, approvedAt: z.number().int() }).strict().nullable()
  .describe("The content approved before, which runs meanwhile (show the person the difference); null when the rule runs nowhere yet.");
const pendingRuleBase = {
  owner: z.enum(RULE_APPROVAL_OWNERS).describe("library: a rule created by you; pack: a rule of a pack added from a folder or tarball."),
  id: z.string(),
  packId: z.string().nullable(),
  revision: z.number().int().positive(),
  contentHash: contentHashField,
  copiedFromRuleId: z.string().nullable().describe("Set for an edited copy of a By Rerail rule: approving it the first time switches the original off where the copy is added."),
};
export const pendingRuleSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("code"), rule: codeRuleSchema, approved: approvedContentSchema(codeRuleSchema), ...pendingRuleBase }).strict(),
  z.object({ kind: z.literal("conversation"), rule: conversationRuleSchema, approved: approvedContentSchema(conversationRuleSchema), ...pendingRuleBase }).strict(),
]);

export const ruleApproveBodySchema = z.object({
  owner: z.enum(RULE_APPROVAL_OWNERS),
  id: z.string().min(1),
  contentHash: contentHashField.describe("The contentHash of the content the person read (rule_approvals_list); a rule that has changed since is refused."),
}).strict();

export const ruleApproveResultSchema = z.object({
  owner: z.enum(RULE_APPROVAL_OWNERS),
  id: z.string(),
  kind: z.enum(RULE_KINDS),
  contentHash: z.string(),
  approvedAt: z.number().int(),
}).strict();

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

export const followGitignoreBodySchema = z.object({
  ...targetShape,
  followGitignore: z.boolean()
    .describe("true: files Git ignores (.gitignore, .git/info/exclude, global excludes) are not audited; tracked files always are. false: only Rerail's own ignores apply; only a person may choose it, never a coding agent."),
}).strict();

export const resetBodySchema = z.object({ ...targetShape }).strict();

export const resetResultSchema = z.object({
  repositoryKey: z.string(),
  sessions: z.number().int().nonnegative().describe("Ended sessions removed together with all of their history."),
  openSessions: z.number().int().nonnegative()
    .describe("Sessions still running: their rows stay so they carry on recording; their history so far is removed."),
}).strict();
