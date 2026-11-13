import { dirname, join, resolve } from "node:path";
import { and, asc, eq, inArray } from "drizzle-orm";
import { z } from "zod";
import { parseRuleDocument } from "./rule-documents.js";
import { validateCodeRule } from "../rules.js";
import { codeRuleValueSchema } from "../code-rule/code-rule-schema.js";
import {
  libraryRules,
  repositories,
  RULE_KINDS,
  ruleAttachments,
  ruleSettings,
} from "../storage/schema.js";
import type { RerailDatabase } from "../storage/database.js";
import { normalizeFolder, RULE_SETTING_ORIGINS, RULE_SETTING_TARGET_KINDS, RULE_SOURCE_KINDS, RULE_SUPPRESSIONS, type RuleCatalog } from "./repository-model.js";
import { approve, isApproved, ruleSubject } from "./rule-approval.js";
import { listLibraryRules, quarantinedLibraryRules, validateLibraryDocument, writeLibraryRule } from "./rule-library.js";
import { RULE_MODES } from "./rule-mode.js";
import type { RuleDocument } from "./rule-store.js";
import { clearSetupReview } from "./setup-review.js";

export const RULES_DUMP_FILE = "rules.json";

/** The dump sits beside the database it describes: `<home>/rules.json`. */
export function defaultRulesDumpPath(databasePath: string): string {
  return join(dirname(resolve(databasePath)), RULES_DUMP_FILE);
}

/**
 * Compile every code rule of a dump in the engine before anything is written, so an import is all or nothing:
 * the error lists each rule the engine rejects, by dump index and id. A rule whose language is not supported yet
 * imports, inactive.
 */
function assertDumpCompiles(path: string, documents: readonly RuleDocument[]): void {
  const problems: string[] = [];
  for (const [index, document] of documents.entries()) {
    if (document.kind !== "code") continue;
    const validation = validateCodeRule(document.rule);
    if (!validation.ok) problems.push(`  rules[${index}] (${document.rule.id}): ${validation.reason}`);
  }
  if (problems.length > 0) {
    throw new Error(`${path}: nothing imported; ${problems.length} code rule(s) do not compile:\n${problems.join("\n")}`);
  }
}

/** The machine rules dump: the library and each repository's configuration. */
export const RULES_DUMP_VERSION = 1 as const;

const libraryEntrySchema = z.object({
  kind: z.enum(RULE_KINDS),
  rule: z.unknown(),
  revision: z.number().int().positive(),
  copiedFromRuleId: z.string().nullable(),
  createdAt: z.number().int(),
  updatedAt: z.number().int(),
}).strict();

const repositoryEntrySchema = z.object({
  repositoryKey: z.string().min(1),
  displayName: z.string(),
  aliases: z.array(z.string()),
  configuredAt: z.number().int().nullable(),
  attachments: z.array(z.object({
    folder: z.string(),
    sourceKind: z.enum(RULE_SOURCE_KINDS),
    sourceId: z.string().min(1),
  }).strict()),
  // `suppressions`, `config` and `origin` are written only when set, so a dump without them reads in builds that predate
  // them. `origin` keeps a setting the lint config gave as the lint config's, for `rerail packs calibrate`.
  settings: z.array(z.object({
    folder: z.string(),
    targetKind: z.enum(RULE_SETTING_TARGET_KINDS),
    targetId: z.string().min(1),
    on: z.boolean().nullable(),
    mode: z.enum(RULE_MODES).nullable(),
    suppressions: z.enum(RULE_SUPPRESSIONS).nullable().optional(),
    config: z.record(z.string(), codeRuleValueSchema).nullable().optional(),
    origin: z.enum(RULE_SETTING_ORIGINS).nullable().optional(),
  }).strict()
    .refine(
      setting => setting.on !== null || setting.mode !== null || (setting.suppressions ?? null) !== null || (setting.config ?? null) !== null,
      "a setting sets on, mode, suppressions or config",
    )
    .refine(
      setting => (setting.config ?? null) === null || (setting.targetKind === "rule" && setting.folder === ""),
      "config is set for one rule over the whole repository (folder \"\")",
    )),
}).strict();

const rulesDumpSchema = z.object({
  version: z.literal(RULES_DUMP_VERSION),
  exportedAt: z.string(),
  library: z.array(libraryEntrySchema),
  repositories: z.array(repositoryEntrySchema),
}).strict();

export type RulesDump = z.infer<typeof rulesDumpSchema>;

export interface RulesDumpImportOptions {
  /**
   * The dump is the person's own (`rerail rules import --approve`), so each library rule it writes is approved as it is
   * written, as `rerail rules approve` would approve it. Without it, content not approved on this machine runs in Log
   * until a person approves it.
   */
  readonly approve?: boolean;
  readonly now?: number;
}

export interface RulesDumpImportReport {
  readonly library: number;
  readonly repositories: number;
  /**
   * What the dump puts on `block`, each once in dump order: library rules by id, and repository settings by their
   * target (`<rule id>`, or `pack <pack id>`). Block steers until block enforcement lands, so the importer says so.
   */
  readonly onBlock: readonly string[];
  /** The library rules the import approved (`approve`): those whose imported content was not approved already. */
  readonly approved: readonly string[];
  /** The library rules whose imported content is not approved here, so they run in Log until a person approves them. */
  readonly waiting: readonly string[];
}

/**
 * Export the library and every repository's configuration; checkout settings and start state stay on this machine. A
 * setting that sets neither `on` nor `mode` (only fields a later Rerail stored) is left out: every setting of a dump
 * sets one of them, and the import refuses one that does not.
 */
export function exportRulesDump(db: RerailDatabase, now = new Date()): RulesDump {
  const quarantined = quarantinedLibraryRules(db);
  if (quarantined.length > 0) throw new Error(`cannot export ${quarantined.length} quarantined library rule(s)`);
  const library = listLibraryRules(db).map(rule => ({
    kind: rule.document.kind,
    rule: rule.document.rule as unknown,
    revision: rule.revision,
    copiedFromRuleId: rule.copiedFromRuleId,
    createdAt: rule.createdAt,
    updatedAt: rule.updatedAt,
  }));
  const attachments = db.select().from(ruleAttachments)
    .orderBy(asc(ruleAttachments.repositoryKey), asc(ruleAttachments.folder), asc(ruleAttachments.sourceKind), asc(ruleAttachments.sourceId)).all();
  const settings = db.select().from(ruleSettings).where(eq(ruleSettings.ownerKind, "repository"))
    .orderBy(asc(ruleSettings.ownerId), asc(ruleSettings.folder), asc(ruleSettings.targetKind), asc(ruleSettings.targetId)).all();
  const repositoryRows = db.select().from(repositories).orderBy(asc(repositories.repositoryKey)).all();
  return {
    version: RULES_DUMP_VERSION,
    exportedAt: now.toISOString(),
    library,
    repositories: repositoryRows.map(row => ({
      repositoryKey: row.repositoryKey,
      displayName: row.displayName,
      aliases: row.aliases,
      configuredAt: row.configuredAt,
      attachments: attachments.filter(attachment => attachment.repositoryKey === row.repositoryKey)
        .map(({ folder, sourceKind, sourceId }) => ({ folder, sourceKind, sourceId })),
      settings: settings.filter(setting => setting.ownerId === row.repositoryKey
        && (setting.on !== null || setting.mode !== null || setting.suppressions !== null || setting.config !== null))
        .map(({ folder, targetKind, targetId, on, mode, suppressions, config, origin }) => ({
          folder, targetKind, targetId, on, mode,
          ...(suppressions === null ? {} : { suppressions }),
          ...(config === null ? {} : { config }),
          ...(origin === null ? {} : { origin }),
        })),
    })),
  };
}

/**
 * Import a rules dump, all or nothing: every code rule must compile first. Library rules are upserted, including one
 * that takes a By Rerail rule's id (the library rule wins). Each listed repository's attachments and repository settings
 * are replaced and its set-up time kept, while this machine's start state is left alone; a repository this machine
 * knows under another key or an alias receives them, and keeps every key as an alias. Approvals are this machine's
 * (`rule-approval.ts`): an imported rule whose content was not approved here runs in Log until a person approves it, or
 * is approved by the import itself with `approve`.
 */
export function importRulesDump(db: RerailDatabase, ruleCatalog: RuleCatalog, value: unknown, options: RulesDumpImportOptions = {}): RulesDumpImportReport {
  const now = options.now ?? Date.now();
  const version = typeof value === "object" && value !== null ? (value as { version?: unknown }).version : undefined;
  if (version !== RULES_DUMP_VERSION) {
    throw new Error(`a v${String(version)} rules dump can't be imported; export it again with this Rerail (v${RULES_DUMP_VERSION})`);
  }
  const dump = rulesDumpSchema.parse(value);
  const documents = dump.library.map((entry, index) => {
    const document = parseRuleDocument(entry.rule, `rules dump library[${index}]`);
    if (document.kind !== entry.kind) throw new Error(`rules dump library[${index}] body does not match kind ${entry.kind}`);
    if (document.kind === "conversation") validateLibraryDocument(document.rule, document.kind);
    return { document, entry };
  });
  assertDumpCompiles("rules dump", documents.map(({ document }) => document));
  const approved: string[] = [];
  const waiting: string[] = [];
  db.transaction(tx => {
    for (const { document, entry } of documents) {
      writeLibraryRule(tx, ruleCatalog, document, {
        revision: entry.revision,
        copiedFromRuleId: entry.copiedFromRuleId,
        createdAt: entry.createdAt,
        now: entry.updatedAt,
        shadowBundled: true,
      });
    }
    const known = tx.select().from(repositories).all();
    for (const repository of dump.repositories) {
      // A repository this machine knows under another key, or under one of its aliases, takes the dump's configuration.
      const names = [repository.repositoryKey, ...repository.aliases];
      const existing = known.find(row => names.includes(row.repositoryKey) || row.aliases.some(alias => names.includes(alias)));
      const key = existing?.repositoryKey ?? repository.repositoryKey;
      const aliases = [...new Set([...(existing?.aliases ?? []), ...names])].filter(alias => alias !== key);
      const displayName = existing === undefined || existing.repositoryKey === repository.repositoryKey ? repository.displayName : existing.displayName;
      tx.insert(repositories).values({
        repositoryKey: key,
        displayName,
        aliases,
        configuredAt: repository.configuredAt,
        createdAt: now,
        updatedAt: now,
      }).onConflictDoUpdate({
        target: repositories.repositoryKey,
        set: { displayName, aliases, configuredAt: repository.configuredAt, updatedAt: now },
      }).run();
      tx.delete(ruleAttachments).where(eq(ruleAttachments.repositoryKey, key)).run();
      // The dump's configuration is not the review's, so set-up no longer waits on one it offered.
      clearSetupReview(tx, key);
      tx.delete(ruleSettings).where(and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, key))).run();
      for (const attachment of repository.attachments) {
        tx.insert(ruleAttachments).values({ repositoryKey: key, ...attachment, folder: normalizeFolder(attachment.folder), createdAt: now })
          .onConflictDoNothing().run();
      }
      for (const setting of repository.settings) {
        tx.insert(ruleSettings).values({ ownerKind: "repository", ownerId: key, ...setting, folder: normalizeFolder(setting.folder), updatedAt: now })
          .onConflictDoNothing().run();
      }
    }
    // Approved last, as `rerail rules approve` would right after the import: an edited copy of a By Rerail rule approved
    // the first time switches its original off where the imported configuration adds it.
    const stored = documents.length === 0 ? [] : tx.select({ ruleId: libraryRules.ruleId, payloadHash: libraryRules.payloadHash }).from(libraryRules)
      .where(inArray(libraryRules.ruleId, documents.map(({ document }) => document.rule.id))).orderBy(asc(libraryRules.ruleId)).all();
    for (const { ruleId, payloadHash } of stored) {
      const subject = ruleSubject(ruleId, payloadHash);
      if (isApproved(tx, subject)) continue;
      if (options.approve !== true) {
        waiting.push(ruleId);
        continue;
      }
      approve(tx, subject, now);
      approved.push(ruleId);
    }
  }, { behavior: "immediate" });
  const onBlock = new Set([
    ...documents.filter(({ document }) => document.rule.mode === "block").map(({ document }) => document.rule.id),
    ...dump.repositories.flatMap(repository => repository.settings.filter(setting => setting.mode === "block")
      .map(setting => (setting.targetKind === "pack" ? `pack ${setting.targetId}` : setting.targetId))),
  ]);
  return { library: documents.length, repositories: dump.repositories.length, onBlock: [...onBlock], approved, waiting };
}
