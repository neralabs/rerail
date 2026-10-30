/**
 * The rule library: rules people wrote ("Created by you"), stored once per machine and added to
 * repositories by attachments. A library rule can't take a By Rerail rule's id, and one id keeps one kind.
 */
import { and, asc, eq, notInArray, or } from "drizzle-orm";
import type { RerailDatabase } from "../storage/database.js";
import { readStoredDocument, type QuarantinedRule } from "./rule-quarantine.js";
import { storableCodeRule } from "../code-rule/code-rule-compile.js";
import type { RuleCatalog, LibraryRule, RepositoryKey } from "./repository-model.js";
import { approveLibraryContent, storedContentDigest } from "./rule-approval.js";
import { assertEngineRegexes } from "./rule-regexes.js";
import { parseConversationRule } from "./rule-schema.js";
import type { RuleDocument } from "./rule-store.js";
import {
  libraryRules,
  ruleApprovals,
  ruleAttachments,
  ruleCatalogIdentities,
  ruleSettings,
  RULE_KINDS,
  type RuleKind,
} from "../storage/schema.js";
import { ruleDocumentDigest } from "../stable-json.js";

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
  /**
   * The write is a person's own (the dashboard), so its content is approved with it (`rule-approval.ts`). Otherwise it
   * waits for approval, and the content approved before, if any, keeps running.
   */
  readonly approve?: boolean;
}

export interface LibraryRuleQuery {
  readonly kind?: RuleKind;
}

export interface CopiedRule {
  readonly rule: LibraryRule;
  /** Folders of the repository where the copy was added; the original is switched off there once the copy is approved. */
  readonly folders: readonly string[];
}

/** Validate a rule body of `kind`: code rules must compile in the engine (or name a language it does not read yet), conversation regexes must be accepted. */
export function validateLibraryDocument(document: unknown, kind: RuleKind): RuleDocument {
  if (kind === "code") return { kind, rule: storableCodeRule(document).rule };
  const rule = parseConversationRule(document);
  assertEngineRegexes(rule);
  return { kind, rule };
}

/** Write an already validated document; the caller owns the transaction. */
export function writeLibraryRule(db: Queries, ruleCatalog: RuleCatalog, document: RuleDocument, options: LibraryRuleWriteOptions = {}): void {
  const id = document.rule.id;
  const existing = db.select().from(libraryRules).where(eq(libraryRules.ruleId, id)).get();
  // A library rule written before a release bundled its id wins over that By Rerail rule, so it stays editable.
  if (ruleCatalog.documents.has(id) && existing === undefined && options.shadowBundled !== true) {
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
  if (options.approve === true) approveLibraryContent(db, { document, revision, copiedFromRuleId }, now);
}

/** Validate, then store a library rule. Throws on an invalid body, a By Rerail id, or a kind change. */
export function upsertLibraryRule(
  db: RerailDatabase,
  ruleCatalog: RuleCatalog,
  document: unknown,
  kind: RuleKind,
  options: LibraryRuleWriteOptions = {},
): LibraryRule {
  const validated = validateLibraryDocument(document, kind);
  return db.transaction(tx => {
    writeLibraryRule(tx, ruleCatalog, validated, options);
    return readRequired(tx, validated.rule.id);
  }, { behavior: "immediate" });
}

/** A stored row read and validated; a row that fails validation, or whose kind is unknown, is quarantined, never thrown. */
export function readLibraryRow(row: LibraryRow): { ok: true; rule: LibraryRule } | { ok: false; quarantined: QuarantinedRule } {
  const read = readStoredDocument(row.kind, row.document);
  const contentHash = storedContentDigest(row.kind, row.document);
  if (!read.ok) return { ok: false, quarantined: { ruleId: row.ruleId, kind: read.kind, owner: LIBRARY_OWNER, reason: read.reason } };
  if (contentHash === undefined) return { ok: false, quarantined: { ruleId: row.ruleId, kind: row.kind, owner: LIBRARY_OWNER, reason: "the stored body is not JSON" } };
  return {
    ok: true,
    rule: {
      document: read.document,
      revision: row.revision,
      payloadHash: row.payloadHash,
      contentHash,
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

/** Remove a library rule with its attachments, the settings that target it, its approval and its kind lock. */
export function removeLibraryRule(db: RerailDatabase, id: string): boolean {
  return db.transaction(tx => {
    const removed = tx.delete(libraryRules).where(eq(libraryRules.ruleId, id)).returning({ id: libraryRules.ruleId }).get() !== undefined;
    if (!removed) return false;
    tx.delete(ruleAttachments).where(and(eq(ruleAttachments.sourceKind, "library_rule"), eq(ruleAttachments.sourceId, id))).run();
    tx.delete(ruleSettings).where(and(eq(ruleSettings.targetKind, "rule"), eq(ruleSettings.targetId, id))).run();
    tx.delete(ruleApprovals).where(and(eq(ruleApprovals.owner, "library"), eq(ruleApprovals.ruleId, id))).run();
    tx.delete(ruleCatalogIdentities).where(eq(ruleCatalogIdentities.ruleId, id)).run();
    return true;
  }, { behavior: "immediate" });
}

function freeCopyId(db: Queries, ruleCatalog: RuleCatalog, sourceId: string): string {
  const taken = (id: string) => ruleCatalog.documents.has(id)
    || db.select({ id: ruleCatalogIdentities.ruleId }).from(ruleCatalogIdentities).where(eq(ruleCatalogIdentities.ruleId, id)).get() !== undefined
    || db.select({ id: libraryRules.ruleId }).from(libraryRules).where(eq(libraryRules.ruleId, id)).get() !== undefined;
  let candidate = `${sourceId}-copy`;
  for (let suffix = 2; taken(candidate); suffix += 1) candidate = `${sourceId}-copy-${suffix}`;
  return candidate;
}

/**
 * Store an edited By Rerail rule as a library copy, in one transaction: wherever the original is present in the
 * repository (its pack or the rule itself is attached), the copy is added at the same folder. Once the copy's content
 * is approved (`approve`: at once, for a person's own edit), a repository setting switches the original off where the
 * copy is added (`rule-approval.ts`); until then the original keeps running. The original stays a reference to the
 * rule catalog.
 */
export function copyRerailRuleToLibrary(
  db: RerailDatabase,
  ruleCatalog: RuleCatalog,
  repositoryKey: RepositoryKey,
  sourceId: string,
  edited: unknown,
  options: { readonly id?: string; readonly now?: number; readonly approve?: boolean } = {},
): CopiedRule {
  const original = ruleCatalog.documents.get(sourceId);
  if (original === undefined) throw new Error(`${sourceId} is not a By Rerail rule`);
  if (typeof edited !== "object" || edited === null) throw new Error("the edited rule must be an object");
  const now = options.now ?? Date.now();
  return db.transaction(tx => {
    const id = options.id ?? freeCopyId(tx, ruleCatalog, sourceId);
    const document = validateLibraryDocument({ ...edited, id }, original.kind);
    writeLibraryRule(tx, ruleCatalog, document, { copiedFromRuleId: sourceId, now });
    const packId = ruleCatalog.packOf.get(sourceId);
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
    }
    const rule = readRequired(tx, id);
    if (options.approve === true) approveLibraryContent(tx, { document, revision: rule.revision, copiedFromRuleId: sourceId }, now);
    return { rule, folders };
  }, { behavior: "immediate" });
}
