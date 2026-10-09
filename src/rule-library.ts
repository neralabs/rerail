/**
 * The rule library: rules people wrote ("Created by you"), stored once per machine and added to
 * repositories by attachments. A library rule can't take a By Rerail rule's id, and one id keeps one kind.
 */
import { and, asc, eq, notInArray, or } from "drizzle-orm";
import type { RerailDatabase } from "./database.js";
import { readStoredCodeRule, unknownRuleKind, type QuarantinedRule } from "./rule-quarantine.js";
import { assertCodeRuleStorable } from "./code-rule-compile.js";
import { codeRuleSchema } from "./code-rule-schema.js";
import type { BundledCatalog, LibraryRule, RepositoryKey } from "./repository-model.js";
import { assertEngineRegexes } from "./rule-regexes.js";
import { parseConversationRule, type ConversationRule } from "./rule-schema.js";
import type { RuleDocument } from "./rule-store.js";
import {
  libraryRules,
  ruleAttachments,
  ruleCatalogIdentities,
  ruleSettings,
  RULE_KINDS,
  type RuleKind,
} from "./schema.js";
import { ruleDocumentDigest } from "./stable-json.js";

/** Quarantine owner label for library rows. */
export const LIBRARY_OWNER = "library";

type LibraryRow = typeof libraryRules.$inferSelect;
type Queries = Pick<RerailDatabase, "select" | "insert" | "update" | "delete">;

export interface LibraryRuleWriteOptions {
  /** Conversation rules carry no revision; omit to bump the stored one. Code rules take `rule.revision`. */
  readonly revision?: number;
  readonly copiedFromRuleId?: string | null;
  readonly createdAt?: number;
  readonly now?: number;
  /**
   * Store the rule even when it takes a By Rerail rule's id: a rules dump's library rule was written before a release
   * bundled its id, and the library rule wins. Without it only a rule already in the library may keep such an id.
   */
  readonly shadowBundled?: boolean;
}

export interface LibraryRuleQuery {
  readonly kind?: RuleKind;
}

export interface CopiedRule {
  readonly rule: LibraryRule;
  /** Folders of the repository where the copy was added and the original switched off. */
  readonly folders: readonly string[];
}

/** Validate a rule body of `kind`: code rules must compile in the engine (or name a language it does not read yet), conversation regexes must be accepted. */
export function validateLibraryDocument(document: unknown, kind: RuleKind): RuleDocument {
  if (kind === "code") {
    const rule = codeRuleSchema.parse(document);
    assertCodeRuleStorable(rule);
    return { kind, rule };
  }
  const rule = parseConversationRule(document);
  assertEngineRegexes(rule);
  return { kind, rule };
}

/** Write an already validated document; the caller owns the transaction. */
export function writeLibraryRule(db: Queries, bundle: BundledCatalog, document: RuleDocument, options: LibraryRuleWriteOptions = {}): void {
  const id = document.rule.id;
  const existing = db.select().from(libraryRules).where(eq(libraryRules.ruleId, id)).get();
  // A library rule written before a release bundled its id wins over that By Rerail rule, so it stays editable.
  if (bundle.documents.has(id) && existing === undefined && options.shadowBundled !== true) {
    throw new Error(`rule ${id} is a By Rerail rule id; give your rule another id`);
  }
  // A database from before the library table can hold an id's kind with no library row; one of a kind this build
  // does not know (the code rule kind's old name) locks nothing, since no remove could clear it.
  if (existing === undefined) {
    db.delete(ruleCatalogIdentities)
      .where(and(eq(ruleCatalogIdentities.ruleId, id), notInArray(ruleCatalogIdentities.kind, [...RULE_KINDS]))).run();
  }
  db.insert(ruleCatalogIdentities).values({ ruleId: id, kind: document.kind }).onConflictDoNothing().run();
  const identity = db.select().from(ruleCatalogIdentities).where(eq(ruleCatalogIdentities.ruleId, id)).get();
  if (identity?.kind !== document.kind) {
    throw new Error(`rule ${id} exists with kind ${identity?.kind}; remove it before storing ${document.kind}`);
  }
  const now = options.now ?? Date.now();
  let revision: number;
  if (document.kind === "code") revision = document.rule.revision;
  else revision = options.revision ?? (existing === undefined ? 1 : existing.revision + 1);
  const payloadHash = ruleDocumentDigest(document);
  const copiedFromRuleId = options.copiedFromRuleId === undefined ? existing?.copiedFromRuleId ?? null : options.copiedFromRuleId;
  db.insert(libraryRules)
    .values({
      ruleId: id,
      kind: document.kind,
      document,
      revision,
      payloadHash,
      copiedFromRuleId,
      createdAt: existing?.createdAt ?? options.createdAt ?? now,
      updatedAt: now,
    })
    .onConflictDoUpdate({
      target: libraryRules.ruleId,
      set: { kind: document.kind, document, revision, payloadHash, copiedFromRuleId, updatedAt: now },
    })
    .run();
}

/** Validate, then store a library rule. Throws on an invalid body, a By Rerail id, or a kind change. */
export function upsertLibraryRule(
  db: RerailDatabase,
  bundle: BundledCatalog,
  document: unknown,
  kind: RuleKind,
  options: LibraryRuleWriteOptions = {},
): LibraryRule {
  const validated = validateLibraryDocument(document, kind);
  return db.transaction(tx => {
    writeLibraryRule(tx, bundle, validated, options);
    return readRequired(tx, validated.rule.id);
  }, { behavior: "immediate" });
}

/** A stored row read and validated; a row that fails validation, or whose kind is unknown, is quarantined, never thrown. */
export function readLibraryRow(row: LibraryRow): { ok: true; rule: LibraryRule } | { ok: false; quarantined: QuarantinedRule } {
  const quarantine = (reason: string, kind: RuleKind = row.kind) => ({
    ok: false as const,
    quarantined: { ruleId: row.ruleId, kind, owner: LIBRARY_OWNER, reason },
  });
  const unknown = unknownRuleKind(row.kind, row.document);
  if (unknown !== undefined) return quarantine(unknown.reason, unknown.kind);
  let document: RuleDocument;
  if (row.kind === "code") {
    const read = readStoredCodeRule(row.document.rule);
    if (!read.ok) return quarantine(read.reason);
    document = { kind: "code", rule: read.rule };
  } else {
    try {
      document = { kind: "conversation", rule: parseConversationRule(row.document.rule) };
    } catch (error) {
      return quarantine(error instanceof Error ? error.message : String(error));
    }
  }
  return {
    ok: true,
    rule: {
      document,
      revision: row.revision,
      payloadHash: row.payloadHash,
      copiedFromRuleId: row.copiedFromRuleId,
      createdAt: row.createdAt,
      updatedAt: row.updatedAt,
    },
  };
}

function readRequired(db: Queries, id: string): LibraryRule {
  const row = db.select().from(libraryRules).where(eq(libraryRules.ruleId, id)).get();
  const read = row === undefined ? undefined : readLibraryRow(row);
  if (read?.ok !== true) throw new Error(`library rule ${id} could not be read back`);
  return read.rule;
}

export function getLibraryRule(db: RerailDatabase, id: string): LibraryRule | undefined {
  const row = db.select().from(libraryRules).where(eq(libraryRules.ruleId, id)).get();
  if (row === undefined) return undefined;
  const read = readLibraryRow(row);
  return read.ok ? read.rule : undefined;
}

/** Readable library rules, by id; quarantined rows are left out (see {@link quarantinedLibraryRules}). */
export function listLibraryRules(db: RerailDatabase, query: LibraryRuleQuery = {}): LibraryRule[] {
  const rows = db.select().from(libraryRules)
    .where(query.kind === undefined ? undefined : eq(libraryRules.kind, query.kind))
    .orderBy(asc(libraryRules.ruleId)).all();
  return rows.flatMap(row => {
    const read = readLibraryRow(row);
    return read.ok ? [read.rule] : [];
  });
}

export function quarantinedLibraryRules(db: RerailDatabase): QuarantinedRule[] {
  return db.select().from(libraryRules).orderBy(asc(libraryRules.ruleId)).all().flatMap(row => {
    const read = readLibraryRow(row);
    return read.ok ? [] : [read.quarantined];
  });
}

/** Remove a library rule with its attachments, the settings that target it, and its kind lock. */
export function removeLibraryRule(db: RerailDatabase, id: string): boolean {
  return db.transaction(tx => {
    const removed = tx.delete(libraryRules).where(eq(libraryRules.ruleId, id)).returning({ id: libraryRules.ruleId }).get() !== undefined;
    if (!removed) return false;
    tx.delete(ruleAttachments).where(and(eq(ruleAttachments.sourceKind, "library_rule"), eq(ruleAttachments.sourceId, id))).run();
    tx.delete(ruleSettings).where(and(eq(ruleSettings.targetKind, "rule"), eq(ruleSettings.targetId, id))).run();
    tx.delete(ruleCatalogIdentities).where(eq(ruleCatalogIdentities.ruleId, id)).run();
    return true;
  }, { behavior: "immediate" });
}

function freeCopyId(db: Queries, bundle: BundledCatalog, sourceId: string): string {
  const taken = (id: string) => bundle.documents.has(id)
    || db.select({ id: ruleCatalogIdentities.ruleId }).from(ruleCatalogIdentities).where(eq(ruleCatalogIdentities.ruleId, id)).get() !== undefined
    || db.select({ id: libraryRules.ruleId }).from(libraryRules).where(eq(libraryRules.ruleId, id)).get() !== undefined;
  let candidate = `${sourceId}-copy`;
  for (let suffix = 2; taken(candidate); suffix += 1) candidate = `${sourceId}-copy-${suffix}`;
  return candidate;
}

/**
 * Store an edited By Rerail rule as a library copy, in one transaction: wherever the original is present in the
 * repository (its pack or the rule itself is attached), the copy is added at the same folder and a repository
 * setting switches the original off there. The original stays a reference to the bundle.
 */
export function copyRerailRuleToLibrary(
  db: RerailDatabase,
  bundle: BundledCatalog,
  repositoryKey: RepositoryKey,
  sourceId: string,
  edited: unknown,
  options: { readonly id?: string; readonly now?: number } = {},
): CopiedRule {
  const original = bundle.documents.get(sourceId);
  if (original === undefined) throw new Error(`${sourceId} is not a By Rerail rule`);
  if (typeof edited !== "object" || edited === null) throw new Error("the edited rule must be an object");
  const now = options.now ?? Date.now();
  return db.transaction(tx => {
    const id = options.id ?? freeCopyId(tx, bundle, sourceId);
    const document = validateLibraryDocument({ ...edited, id }, original.kind);
    writeLibraryRule(tx, bundle, document, { copiedFromRuleId: sourceId, now });
    const packId = bundle.packOf.get(sourceId);
    const presences = tx.select({ folder: ruleAttachments.folder }).from(ruleAttachments).where(and(
      eq(ruleAttachments.repositoryKey, repositoryKey),
      packId === undefined
        ? and(eq(ruleAttachments.sourceKind, "rerail_rule"), eq(ruleAttachments.sourceId, sourceId))
        : or(
          and(eq(ruleAttachments.sourceKind, "rerail_rule"), eq(ruleAttachments.sourceId, sourceId)),
          and(eq(ruleAttachments.sourceKind, "pack"), eq(ruleAttachments.sourceId, packId)),
        ),
    )).all();
    const folders = [...new Set(presences.map(row => row.folder))].sort();
    for (const folder of folders) {
      tx.insert(ruleAttachments)
        .values({ repositoryKey, folder, sourceKind: "library_rule", sourceId: id, createdAt: now })
        .onConflictDoNothing().run();
      tx.insert(ruleSettings)
        .values({ ownerKind: "repository", ownerId: repositoryKey, folder, targetKind: "rule", targetId: sourceId, on: false, mode: null, updatedAt: now })
        .onConflictDoUpdate({
          target: [ruleSettings.ownerKind, ruleSettings.ownerId, ruleSettings.folder, ruleSettings.targetKind, ruleSettings.targetId],
          set: { on: false, updatedAt: now },
        }).run();
    }
    return { rule: readRequired(tx, id), folders };
  }, { behavior: "immediate" });
}
