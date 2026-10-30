/**
 * Rule approval. A rule runs only with content a person approved, unless Nera shipped it: the rules of a pack installed
 * from the npm registry (`RuleCatalog.shipped`). Every other rule, one created by you or one of a pack added from a
 * folder or tarball, waits for approval when it is new and whenever its content changes. An edit of an approved rule
 * leaves the approved content running while the edit waits, so editing never switches a rule off; a rule with no
 * approved content runs nowhere.
 *
 * An approval binds to the rule's content digest (`ruleContentDigest`: everything but `mode` and `revision`, which are
 * settings) and keeps the approved content itself (`rule_approvals`). Every read recomputes the digest from the stored
 * content, so a row that merely claims an approval approves nothing. Writes through `/api` are a person's own and
 * approve what they store; the CLI and coding agents (MCP) only propose, and a person approves in the dashboard or
 * with `rerail rules approve <id>`.
 */
import { and, asc, eq, inArray } from "drizzle-orm";
import type { RerailDatabase } from "../storage/database.js";
import { libraryRules, ruleApprovals, ruleAttachments, ruleSettings, type RuleApprovalOwner } from "../storage/schema.js";
import { compareDeterministicStrings, ruleContentDigest } from "../stable-json.js";
import type { RuleApprovalStatus, RuleCatalog } from "./repository-model.js";
import { readStoredDocument } from "./rule-quarantine.js";
import type { RuleDocument } from "./rule-store.js";

type Queries = Pick<RerailDatabase, "select" | "insert" | "update" | "delete">;
export type ApprovalRow = typeof ruleApprovals.$inferSelect;

/** Content a person approved for a rule, read back and checked. */
export interface RuleApproval {
  readonly owner: RuleApprovalOwner;
  readonly document: RuleDocument;
  readonly revision: number;
  readonly contentHash: string;
  readonly approvedAt: number;
}

/** The key {@link readApprovals} files an approval under. */
export function approvalKey(owner: RuleApprovalOwner, ruleId: string): string {
  return `${owner}\0${ruleId}`;
}

/** The content digest of a stored `{ kind, rule }` body, or undefined for one that is not JSON representable. */
export function storedContentDigest(kind: string, stored: unknown): string | undefined {
  const rule = typeof stored === "object" && stored !== null && "rule" in stored ? stored.rule : undefined;
  try {
    return ruleContentDigest(kind, rule);
  } catch {
    return undefined;
  }
}

/**
 * An approval row, checked: its stored content must hash to its `content_hash`, carry its rule id and validate as a
 * rule of its kind. Undefined otherwise: such a row approves nothing.
 */
export function readApprovalRow(row: ApprovalRow): RuleApproval | undefined {
  const contentHash = storedContentDigest(row.kind, row.document);
  if (contentHash === undefined || contentHash !== row.contentHash) return undefined;
  const read = readStoredDocument(row.kind, row.document);
  if (!read.ok || read.document.rule.id !== row.ruleId) return undefined;
  return { owner: row.owner, document: read.document, revision: row.revision, contentHash, approvedAt: row.approvedAt };
}

/** The checked approvals of `ruleIds` (every rule's without), by {@link approvalKey}. */
export function readApprovals(db: Pick<RerailDatabase, "select">, ruleIds?: readonly string[]): Map<string, RuleApproval> {
  if (ruleIds?.length === 0) return new Map();
  const rows = db.select().from(ruleApprovals)
    .where(ruleIds === undefined ? undefined : inArray(ruleApprovals.ruleId, [...ruleIds])).all();
  return new Map(rows.flatMap(row => {
    const approval = readApprovalRow(row);
    return approval === undefined ? [] : [[approvalKey(row.owner, row.ruleId), approval] as const];
  }));
}

/** How content whose digest is `contentHash` stands against the rule's approval. */
export function approvalStatus(contentHash: string, approval: RuleApproval | undefined): RuleApprovalStatus {
  if (approval === undefined) return { state: "pending", approvedAt: null };
  return { state: approval.contentHash === contentHash ? "approved" : "pending", approvedAt: approval.approvedAt };
}

/** What a rule that needs approval runs: its current content once approved, else the content approved before. */
export interface ApprovedContent {
  readonly document: RuleDocument;
  readonly revision: number;
  readonly approval: RuleApprovalStatus;
}

/**
 * The content a rule that needs approval runs, given its current content and its approval: the current content when
 * its digest is the approved one, else the approved content, while the current content waits. Undefined when nothing
 * of the rule is approved, or what was is of another kind: it runs nowhere.
 */
export function approvedContent(
  current: { readonly document: RuleDocument; readonly revision: number; readonly contentHash: string },
  approval: RuleApproval | undefined,
): ApprovedContent | undefined {
  if (approval === undefined || approval.document.kind !== current.document.kind) return undefined;
  const status = approvalStatus(current.contentHash, approval);
  return status.state === "approved"
    ? { document: current.document, revision: current.revision, approval: status }
    : { document: approval.document, revision: approval.revision, approval: status };
}

/** How catalog rule `document` stands: as shipped for a rule Nera shipped, else against its approval in `approvals`. */
export function catalogApprovalStatus(ruleCatalog: RuleCatalog, approvals: ReadonlyMap<string, RuleApproval>, document: RuleDocument): RuleApprovalStatus {
  if (ruleCatalog.shipped.has(document.rule.id)) return { state: "shipped", approvedAt: null };
  return approvalStatus(ruleContentDigest(document.kind, document.rule), approvals.get(approvalKey("pack", document.rule.id)));
}

/** The revision a catalog rule is stored and reported with: a code rule's own, 1 for a conversation rule. */
export function catalogRevision(document: RuleDocument): number {
  return document.kind === "code" ? document.rule.revision : 1;
}

function writeApproval(db: Queries, owner: RuleApprovalOwner, document: RuleDocument, revision: number, now: number): void {
  const contentHash = ruleContentDigest(document.kind, document.rule);
  db.insert(ruleApprovals)
    .values({ owner, ruleId: document.rule.id, kind: document.kind, document, revision, contentHash, approvedAt: now })
    .onConflictDoUpdate({
      target: [ruleApprovals.owner, ruleApprovals.ruleId],
      set: { kind: document.kind, document, revision, contentHash, approvedAt: now },
    })
    .run();
}

/**
 * Approve a library rule's content as stored. A copy of a By Rerail rule approved for the first time replaces its
 * original: a repository setting switches the original off wherever the copy is added. The caller owns the transaction.
 */
export function approveLibraryContent(
  db: Queries,
  rule: { readonly document: RuleDocument; readonly revision: number; readonly copiedFromRuleId: string | null },
  now: number,
): void {
  const id = rule.document.rule.id;
  const first = db.select({ id: ruleApprovals.ruleId }).from(ruleApprovals)
    .where(and(eq(ruleApprovals.owner, "library"), eq(ruleApprovals.ruleId, id))).get() === undefined;
  writeApproval(db, "library", rule.document, rule.revision, now);
  if (!first || rule.copiedFromRuleId === null) return;
  const added = db.select({ repositoryKey: ruleAttachments.repositoryKey, folder: ruleAttachments.folder }).from(ruleAttachments)
    .where(and(eq(ruleAttachments.sourceKind, "library_rule"), eq(ruleAttachments.sourceId, id))).all();
  for (const { repositoryKey, folder } of added) {
    db.insert(ruleSettings)
      .values({ ownerKind: "repository", ownerId: repositoryKey, folder, targetKind: "rule", targetId: rule.copiedFromRuleId, on: false, mode: null, updatedAt: now })
      .onConflictDoUpdate({
        target: [ruleSettings.ownerKind, ruleSettings.ownerId, ruleSettings.folder, ruleSettings.targetKind, ruleSettings.targetId],
        set: { on: false, updatedAt: now },
      }).run();
  }
}

/** A rule whose current content waits for a person's approval. */
export interface PendingRule {
  readonly owner: RuleApprovalOwner;
  /** The pack that holds a pack rule; null for a rule created by you. */
  readonly packId: string | null;
  /** The content that waits. */
  readonly document: RuleDocument;
  readonly revision: number;
  /** Its content digest: what `approveRule` must be given, so a person approves the content they read. */
  readonly contentHash: string;
  /** Set when the rule is an edited copy of a By Rerail rule, which approving it switches off where it is added. */
  readonly copiedFromRuleId: string | null;
  /** The content approved before, which runs meanwhile; undefined when the rule runs nowhere yet. */
  readonly approved: RuleApproval | undefined;
}

/**
 * Every rule whose current content waits for approval, by id: rules created by you (a row that fails validation is
 * quarantined and left out) and rules of packs Nera did not ship.
 */
export function pendingRules(db: RerailDatabase, ruleCatalog: RuleCatalog): PendingRule[] {
  const approvals = readApprovals(db);
  const pending: PendingRule[] = [];
  for (const row of db.select().from(libraryRules).orderBy(asc(libraryRules.ruleId)).all()) {
    const read = readStoredDocument(row.kind, row.document);
    const contentHash = storedContentDigest(row.kind, row.document);
    if (!read.ok || contentHash === undefined) continue;
    const approved = approvals.get(approvalKey("library", row.ruleId));
    if (approved?.contentHash === contentHash) continue;
    pending.push({ owner: "library", packId: null, document: read.document, revision: row.revision, contentHash, copiedFromRuleId: row.copiedFromRuleId, approved });
  }
  for (const document of ruleCatalog.documents.values()) {
    const id = document.rule.id;
    if (ruleCatalog.shipped.has(id)) continue;
    const contentHash = ruleContentDigest(document.kind, document.rule);
    const approved = approvals.get(approvalKey("pack", id));
    if (approved?.contentHash === contentHash) continue;
    pending.push({ owner: "pack", packId: ruleCatalog.packOf.get(id) ?? null, document, revision: catalogRevision(document), contentHash, copiedFromRuleId: null, approved });
  }
  return pending.sort((left, right) => compareDeterministicStrings(left.document.rule.id, right.document.rule.id)
    || compareDeterministicStrings(left.owner, right.owner));
}

/** Why `approveRule` refused. */
export class RuleApprovalError extends Error {
  constructor(readonly code: "rule-not-found" | "rule-changed" | "approval-not-needed", message: string) {
    super(message);
    this.name = "RuleApprovalError";
  }
}

/**
 * Approve the current content of rule `ruleId` (`owner` `library`: created by you; `pack`: of a pack Nera did not
 * ship), which must have the content digest `contentHash`: the content the person read, not whatever it has become
 * since. From then on the rule runs with it. Throws {@link RuleApprovalError}.
 */
export function approveRule(
  db: RerailDatabase,
  ruleCatalog: RuleCatalog,
  owner: RuleApprovalOwner,
  ruleId: string,
  contentHash: string,
  now: number = Date.now(),
): RuleApproval {
  return db.transaction(tx => {
    const changed = () => new RuleApprovalError("rule-changed", `rule ${ruleId} changed since it was read; read it again and approve what it says now`);
    if (owner === "library") {
      const row = tx.select().from(libraryRules).where(eq(libraryRules.ruleId, ruleId)).get();
      if (row === undefined) throw new RuleApprovalError("rule-not-found", `library rule ${ruleId} does not exist`);
      const read = readStoredDocument(row.kind, row.document);
      if (!read.ok) throw new RuleApprovalError("rule-not-found", `library rule ${ruleId} cannot be read: ${read.reason}`);
      if (storedContentDigest(row.kind, row.document) !== contentHash) throw changed();
      approveLibraryContent(tx, { document: row.document, revision: row.revision, copiedFromRuleId: row.copiedFromRuleId }, now);
    } else {
      const document = ruleCatalog.documents.get(ruleId);
      if (document === undefined) throw new RuleApprovalError("rule-not-found", `no installed pack holds rule ${ruleId}`);
      if (ruleCatalog.shipped.has(ruleId)) {
        throw new RuleApprovalError("approval-not-needed", `rule ${ruleId} comes from a pack installed from the npm registry, which needs no approval`);
      }
      if (ruleContentDigest(document.kind, document.rule) !== contentHash) throw changed();
      writeApproval(tx, "pack", document, catalogRevision(document), now);
    }
    const row = tx.select().from(ruleApprovals).where(and(eq(ruleApprovals.owner, owner), eq(ruleApprovals.ruleId, ruleId))).get();
    const approval = row === undefined ? undefined : readApprovalRow(row);
    if (approval === undefined) throw new Error(`the approval of rule ${ruleId} could not be read back`);
    return approval;
  }, { behavior: "immediate" });
}
