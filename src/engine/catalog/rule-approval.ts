/**
 * Rule approvals: rules Rerail did not publish steer only once a person approved their content. A rule created by you
 * (the library), a rule in a worktree's `.rerail/rules`, and every rule of a pack Rerail does not publish from npm (a
 * third party's, or one installed from a folder or tarball) runs in Log only until its content is approved; an edit is
 * new content, and waits again. Approval is of content, once per machine: a rule by its id at the digest of its
 * document, a pack by its package name at the digest of everything it ships, so a pack is approved once per version.
 * Rerail's own packs from npm need none. An edited copy of a By Rerail rule replaces its original only once approved:
 * until then the original keeps running where the copy is added, so writing a copy never weakens a rule.
 */
import { createHash } from "node:crypto";
import { and, eq, gte } from "drizzle-orm";
import type { RerailDatabase } from "../storage/database.js";
import { libraryRules, ruleApprovals, ruleAttachments, ruleSettings, RULE_APPROVAL_SUBJECTS } from "../storage/schema.js";
import { compareDeterministicStrings, ruleDocumentDigest, stableJsonText } from "../stable-json.js";
import type { EffectiveRule, RuleCatalog } from "./repository-model.js";
import { RERAIL_PUBLISHER, type RulePack } from "./rule-packs.js";
import { isWorktreePack } from "./worktree-pack.js";

export type RuleApprovalSubjectKind = (typeof RULE_APPROVAL_SUBJECTS)[number];

/** What a person approves: one rule's content, or one pack's, at `digest`. */
export interface RuleApprovalSubject {
  readonly kind: RuleApprovalSubjectKind;
  /** The rule's id, or the pack's package name. */
  readonly id: string;
  readonly digest: string;
}

/** An approval subject's key in a set of approvals. */
export const approvalKey = (subject: RuleApprovalSubject): string => `${subject.kind}\0${subject.id}\0${subject.digest}`;

/**
 * Whether `pack`'s rules wait for approval as a pack: every pack but Rerail's own from npm. A worktree's own rules
 * are approved one by one, as rules (`ruleSubject`).
 */
export function packNeedsApproval(pack: RulePack): boolean {
  return !isWorktreePack(pack) && (pack.publisher !== RERAIL_PUBLISHER || pack.localSource !== undefined);
}

const packDigests = new WeakMap<RuleCatalog, Map<string, string>>();

/**
 * The digest of everything `pack` ships, in `catalog`: its package and version, what it says of itself where a person
 * reads it (its name in a steer's note and on the dashboard, its category, what it covers and why it fits), its
 * `pack.json` entries for its rules and their documents. It changes with a new version, or any edit to a pack installed
 * from a folder.
 */
export function packApprovalDigest(catalog: RuleCatalog, pack: RulePack): string {
  let byPack = packDigests.get(catalog);
  if (byPack === undefined) {
    byPack = new Map();
    packDigests.set(catalog, byPack);
  }
  const known = byPack.get(pack.id);
  if (known !== undefined) return known;
  const documents = [...pack.rules].sort((left, right) => compareDeterministicStrings(left.id, right.id)).map(rule => {
    const document = catalog.documents.get(rule.id);
    return { entry: rule, document: document === undefined ? null : ruleDocumentDigest(document) };
  });
  const text = stableJsonText({
    packageName: pack.packageName,
    version: pack.version,
    name: pack.name,
    category: pack.category,
    covers: pack.covers,
    reason: pack.reason,
    honourSuppressionComments: pack.honourSuppressionComments,
    rules: documents,
  }) ?? "";
  const digest = `sha256:${createHash("sha256").update(text).digest("hex")}`;
  byPack.set(pack.id, digest);
  return digest;
}

/** The approval a pack waits for, or undefined for one that needs none. */
export function packSubject(catalog: RuleCatalog, pack: RulePack): RuleApprovalSubject | undefined {
  if (!packNeedsApproval(pack)) return undefined;
  return { kind: "pack", id: pack.packageName ?? pack.id, digest: packApprovalDigest(catalog, pack) };
}

/** The approval a rule's content waits for: by its id, at the digest of the document as written. */
export function ruleSubject(id: string, payloadHash: string): RuleApprovalSubject {
  return { kind: "rule", id, digest: payloadHash };
}

/** Every approval recorded, as keys (`approvalKey`). */
export function readApprovals(db: Pick<RerailDatabase, "select">): ReadonlySet<string> {
  return new Set(db.select().from(ruleApprovals).all()
    .map(row => approvalKey({ kind: row.subjectKind, id: row.subjectId, digest: row.digest })));
}

/**
 * What approving `name` would approve now, in `catalog` (a checkout's, its own rules included): a rule created by you
 * or a worktree's own rule by its id, at its content now; a pack Rerail does not publish by its package name or id, or
 * the pack of a rule of one, at what it ships now. Undefined for a name nothing waits on: Rerail's own pack or rule,
 * or nothing known by it.
 */
export function currentSubject(db: Pick<RerailDatabase, "select">, catalog: RuleCatalog, name: string): RuleApprovalSubject | undefined {
  const library = db.select({ payloadHash: libraryRules.payloadHash }).from(libraryRules).where(eq(libraryRules.ruleId, name)).get();
  if (library !== undefined) return ruleSubject(name, library.payloadHash);
  const pack = [...catalog.packs.values()].find(candidate => candidate.packageName === name || candidate.id === name)
    ?? catalog.packs.get(catalog.packOf.get(name) ?? "");
  if (pack === undefined) return undefined;
  if (!isWorktreePack(pack)) return packSubject(catalog, pack);
  const document = catalog.documents.get(name);
  return document === undefined ? undefined : ruleSubject(name, ruleDocumentDigest(document));
}

/** Whether `subject` was approved. */
export function isApproved(db: Pick<RerailDatabase, "select">, subject: RuleApprovalSubject): boolean {
  return db.select({ id: ruleApprovals.subjectId }).from(ruleApprovals).where(and(
    eq(ruleApprovals.subjectKind, subject.kind), eq(ruleApprovals.subjectId, subject.id), eq(ruleApprovals.digest, subject.digest),
  )).get() !== undefined;
}

/**
 * Record that a person approved `subject`; approving it again changes nothing. The first approval of an edited copy of
 * a By Rerail rule since it was created switches the original off wherever the copy is added, with a repository
 * setting, as the copy now replaces it there.
 */
export function approve(db: Pick<RerailDatabase, "select" | "insert">, subject: RuleApprovalSubject, now: number = Date.now()): void {
  const copy = subject.kind === "rule"
    ? db.select({ copiedFromRuleId: libraryRules.copiedFromRuleId, createdAt: libraryRules.createdAt }).from(libraryRules)
      .where(eq(libraryRules.ruleId, subject.id)).get()
    : undefined;
  const first = copy?.copiedFromRuleId != null && db.select({ id: ruleApprovals.subjectId }).from(ruleApprovals).where(and(
    eq(ruleApprovals.subjectKind, "rule"), eq(ruleApprovals.subjectId, subject.id), gte(ruleApprovals.approvedAt, copy.createdAt),
  )).get() === undefined;
  db.insert(ruleApprovals)
    .values({ subjectKind: subject.kind, subjectId: subject.id, digest: subject.digest, approvedAt: now })
    .onConflictDoNothing()
    .run();
  if (!first) return;
  const added = db.select({ repositoryKey: ruleAttachments.repositoryKey, folder: ruleAttachments.folder }).from(ruleAttachments)
    .where(and(eq(ruleAttachments.sourceKind, "library_rule"), eq(ruleAttachments.sourceId, subject.id))).all();
  for (const { repositoryKey, folder } of added) {
    db.insert(ruleSettings)
      .values({ ownerKind: "repository", ownerId: repositoryKey, folder, targetKind: "rule", targetId: copy.copiedFromRuleId!, on: false, mode: null, updatedAt: now })
      .onConflictDoUpdate({
        target: [ruleSettings.ownerKind, ruleSettings.ownerId, ruleSettings.folder, ruleSettings.targetKind, ruleSettings.targetId],
        set: { on: false, updatedAt: now },
      })
      .run();
  }
}

/** How many rules and packs (each counted once) among `rules` are on and run in Log, waiting for a person's approval. */
export function awaitingApprovalOf(rules: readonly EffectiveRule[]): number {
  return new Set(rules.flatMap(rule => (rule.on && rule.approval?.approved === false
    ? [`${rule.approval.subject.kind}\0${rule.approval.subject.id}`]
    : []))).size;
}
