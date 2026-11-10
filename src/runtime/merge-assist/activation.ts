/**
 * Which candidate rules steer one checkout, and the rule configuration that makes them do so there and nowhere else
 * (docs/merge-assist-v1-build.md §9.2). A candidate's code rule is stored as a library rule, attached at the top of the
 * repository with the repository's own setting off, and switched on for the one checkout: a checkout setting wins over
 * the repository's. A checkout inherits its parent checkout's settings, so the checkouts recorded inside it get an
 * explicit off setting for each of its candidate rules, and it gets one for each of its parents'. A checkout's set is
 * its own active candidates that carry a rule, plus the other branches' candidates it absorbed or adopted. Nothing
 * applies while Rerail is not set up and started in the checkout: Merge Assist never sets a repository up.
 *
 * Only rules whose id is `candidate:<slug>~<tag>` are ever written or removed here. The tag is of the checkout (and,
 * for another branch's candidate, of where it came from and its exact version), so no other checkout, repository or
 * branch can replace a rule this checkout applied by reusing its candidate id: the rule library is shared by all.
 */
import { createHash } from "node:crypto";
import { z } from "zod";
import { and, eq, inArray, sql, type SQLWrapper } from "drizzle-orm";
import type { RerailDatabase } from "../../engine/storage/database.js";
import {
  checkouts,
  libraryRules,
  mergeAssistItems,
  ruleAttachments,
  ruleSettings,
  type MergeAssistItemRow,
} from "../../engine/storage/schema.js";
import type { CheckoutContext, RepositoryKey, RuleCatalog } from "../../engine/catalog/repository-model.js";
import { checkoutStatus, mergeAssistEnabled, nestedCheckouts, parentCheckouts } from "../../engine/catalog/repository-registry.js";
import { addAttachment, setRuleSetting } from "../../engine/catalog/rule-configuration.js";
import { removeLibraryRule, validateLibraryDocument, writeLibraryRule } from "../../engine/catalog/rule-library.js";
import type { RuleDocument } from "../../engine/catalog/rule-store.js";
import type { CodeRule } from "../../engine/code-rule/code-rule-schema.js";
import { candidatesInForce } from "../../engine/merge-assist/candidate.js";
import { readWorktreeCandidates } from "../../engine/merge-assist/candidate-files.js";
import { ruleDocumentDigest } from "../../engine/stable-json.js";
import { provisionalResults } from "./ledger-store.js";
import { STATE_KEYS, deleteState, readState, writeState, type StateQueries } from "./state.js";

/** The id prefix of every rule Merge Assist stores; it touches no other rule. */
export const CANDIDATE_RULE_PREFIX = "candidate:";

/** A rule id column holding a Merge Assist id, `candidate:…~<8 hex>`; GLOB, unlike LIKE, is case-sensitive. */
function candidateRule(column: SQLWrapper) {
  return sql`${column} GLOB ${`${CANDIDATE_RULE_PREFIX}*~${"[0-9a-f]".repeat(8)}`}`;
}

/** The 8-hex tag that ends a Merge Assist rule id. */
function ruleTag(text: string): string {
  return createHash("sha256").update(text).digest("hex").slice(0, 8);
}

/** The library rule id of one of the checkout's own candidates: its id, `~`, and a tag of the checkout. */
export function ownRuleId(checkoutPath: string, candidateId: string): string {
  return `${candidateId}~${ruleTag(`own:${checkoutPath}`)}`;
}

/**
 * The library rule id of another branch's candidate as one checkout applies it: its id, `~`, and a tag of the
 * checkout, the branch or pull request it came from, and its exact version.
 */
export function peerRuleId(
  checkoutPath: string,
  item: Pick<MergeAssistItemRow, "candidateId" | "peerKind" | "peerRef" | "candidateHash">,
): string {
  return `${item.candidateId}~${ruleTag(`peer:${checkoutPath}:${item.peerKind ?? ""}:${item.peerRef}:${item.candidateHash}`)}`;
}

/** The library rule id an item's candidate is stored under in its checkout. */
export function itemRuleId(item: MergeAssistItemRow): string {
  return item.origin === "own" ? ownRuleId(item.checkoutPath, item.candidateId) : peerRuleId(item.checkoutPath, item);
}

/** Whether candidate rules can steer the checkout: Rerail is set up and started there. */
export function rulesActive(db: RerailDatabase, checkout: CheckoutContext): boolean {
  const status = checkoutStatus(db, checkout);
  return status.started && status.setUp;
}

interface WantedRule {
  readonly ruleId: string;
  readonly rule: CodeRule;
}

/**
 * The rules the checkout's candidates call for: its own in force with a rule, and the peers' it applied, but not
 * another branch's verdict result that has not merged.
 */
async function wantedRules(db: RerailDatabase, checkout: CheckoutContext, decisionLedger: boolean): Promise<WantedRule[]> {
  const root = checkout.checkoutPath;
  const own = await readWorktreeCandidates(root);
  const wanted: WantedRule[] = candidatesInForce(own.candidates)
    .filter(candidate => candidate.rule !== undefined)
    .map(candidate => {
      const ruleId = ownRuleId(root, candidate.id);
      return { ruleId, rule: { ...(candidate.rule as CodeRule), id: ruleId } };
    });
  const ownIds = new Set(own.candidates.map(candidate => candidate.id));
  const applied = db.select().from(mergeAssistItems).where(and(
    eq(mergeAssistItems.checkoutPath, root),
    eq(mergeAssistItems.origin, "peer"),
    eq(mergeAssistItems.status, "applied"),
  )).all();
  // Another branch's verdict that has not merged is information, never a rule here.
  const verdicts = provisionalResults(db, root, decisionLedger);
  for (const item of applied) {
    const rule = item.candidate.rule as CodeRule | undefined;
    if (rule === undefined || ownIds.has(item.candidateId) || (verdicts.get(item.candidateId)?.place ?? "branch") !== "branch") continue;
    const ruleId = peerRuleId(root, item);
    wanted.push({ ruleId, rule: { ...rule, id: ruleId } });
  }
  return wanted;
}

/** The Merge Assist rules the checkouts at `paths` set at their top folder, with whether each is on. */
function checkoutCandidateSettings(tx: StateQueries, paths: readonly string[]): Array<{ ownerId: string; targetId: string; on: boolean | null }> {
  return tx.select({ ownerId: ruleSettings.ownerId, targetId: ruleSettings.targetId, on: ruleSettings.on }).from(ruleSettings).where(and(
    eq(ruleSettings.ownerKind, "checkout"),
    inArray(ruleSettings.ownerId, [...paths]),
    eq(ruleSettings.folder, ""),
    eq(ruleSettings.targetKind, "rule"),
    candidateRule(ruleSettings.targetId),
  )).all();
}

const failuresSchema = z.record(z.string(), z.string());

/** The candidate rules the checkout's last reconciliation could not store, by rule id, with why. */
export function activationFailures(db: StateQueries, checkoutPath: string): Readonly<Record<string, string>> {
  return readState(db, STATE_KEYS.activation(checkoutPath), failuresSchema) ?? {};
}

export interface ActivationOutcome {
  /** The rule ids now on in the checkout. */
  readonly applied: readonly string[];
  /** Rules that could not be stored (they do not compile here, or their id is taken), by rule id, with why. */
  readonly failed: Readonly<Record<string, string>>;
}

/**
 * Make the rule configuration match what the checkout's candidates call for: store and switch on every wanted rule,
 * switch off in this checkout every Merge Assist rule no longer wanted, and remove a rule from the repository (and
 * from the library, when no repository has it) once no checkout of the repository has it on. The checkout's own
 * wanted set wins over what it would inherit: a parent checkout's candidate rule is switched off here, and this
 * checkout's rules are switched off in the checkouts recorded inside it. With Merge Assist off or Rerail not set up
 * and started here, nothing is wanted.
 */
export async function reconcileCheckout(
  db: RerailDatabase,
  ruleCatalog: RuleCatalog,
  checkout: CheckoutContext,
  now: number,
  decisionLedger: boolean,
): Promise<ActivationOutcome> {
  const active = mergeAssistEnabled(db, checkout.repositoryKey) && rulesActive(db, checkout);
  const wanted = active ? await wantedRules(db, checkout, decisionLedger) : [];
  const failed: Record<string, string> = {};
  const documents: RuleDocument[] = [];
  for (const { ruleId, rule } of wanted) {
    try {
      documents.push(validateLibraryDocument(rule, "code"));
    } catch (error) {
      failed[ruleId] = error instanceof Error ? error.message : String(error);
    }
  }
  const applied = db.transaction(tx => {
    const on: string[] = [];
    for (const document of documents) {
      const ruleId = document.rule.id;
      try {
        storeRule(tx, ruleCatalog, document, now);
      } catch (error) {
        failed[ruleId] = error instanceof Error ? error.message : String(error);
        continue;
      }
      addAttachment(tx, { repositoryKey: checkout.repositoryKey, folder: "", sourceKind: "library_rule", sourceId: ruleId }, now);
      const repositorySetting = tx.select({ ownerId: ruleSettings.ownerId }).from(ruleSettings).where(and(
        eq(ruleSettings.ownerKind, "repository"),
        eq(ruleSettings.ownerId, checkout.repositoryKey),
        eq(ruleSettings.folder, ""),
        eq(ruleSettings.targetKind, "rule"),
        eq(ruleSettings.targetId, ruleId),
      )).get();
      if (repositorySetting === undefined) {
        setRuleSetting(tx, {
          owner: { kind: "repository", repositoryKey: checkout.repositoryKey }, folder: "", targetKind: "rule", targetId: ruleId, on: false, mode: null,
        }, now);
      }
      setRuleSetting(tx, {
        owner: { kind: "checkout", checkoutPath: checkout.checkoutPath }, folder: "", targetKind: "rule", targetId: ruleId, on: true, mode: null,
      }, now);
      on.push(ruleId);
    }
    const keep = new Set(on);
    const root = checkout.checkoutPath;
    const parents = parentCheckouts(tx, checkout).map(row => row.checkoutPath);
    const nested = nestedCheckouts(tx, checkout).map(row => row.checkoutPath);
    const settings = new Map(checkoutCandidateSettings(tx, [root, ...parents, ...nested]).map(row => [`${row.ownerId}\0${row.targetId}`, row.on]));
    const off = (path: string, ruleId: string): void => {
      if (settings.get(`${path}\0${ruleId}`) === false) return;
      setRuleSetting(tx, { owner: { kind: "checkout", checkoutPath: path }, folder: "", targetKind: "rule", targetId: ruleId, on: false, mode: null }, now);
    };
    // A parent's candidate rule would reach this checkout through its settings: an explicit off here stops it.
    const shielded = new Set<string>();
    for (const [key, isOn] of settings) {
      const [owner, ruleId] = key.split("\0") as [string, string];
      if (isOn === true && parents.includes(owner) && !keep.has(ruleId)) shielded.add(ruleId);
    }
    for (const ruleId of shielded) off(root, ruleId);
    const stale = [...settings.keys()]
      .map(key => key.split("\0") as [string, string])
      .filter(([owner, ruleId]) => owner === root && !keep.has(ruleId) && !shielded.has(ruleId))
      .map(([, ruleId]) => ruleId);
    if (stale.length > 0) {
      tx.delete(ruleSettings).where(and(
        eq(ruleSettings.ownerKind, "checkout"),
        eq(ruleSettings.ownerId, root),
        eq(ruleSettings.targetKind, "rule"),
        inArray(ruleSettings.targetId, stale),
      )).run();
    }
    // And this checkout's rules would reach the checkouts inside it, unless one of them wants the rule itself.
    for (const path of nested) {
      for (const ruleId of on) if (settings.get(`${path}\0${ruleId}`) !== true) off(path, ruleId);
    }
    removeUnused(db, tx, checkout.repositoryKey);
    if (Object.keys(failed).length === 0) deleteState(tx, STATE_KEYS.activation(checkout.checkoutPath));
    else writeState(tx, STATE_KEYS.activation(checkout.checkoutPath), failed, now);
    return on;
  }, { behavior: "immediate" });
  return { applied, failed };
}

/** Store `document` unless the library already holds exactly it, so an unchanged rule does not reload anywhere. */
function storeRule(tx: StateQueries, ruleCatalog: RuleCatalog, document: RuleDocument, now: number): void {
  const stored = tx.select({ payloadHash: libraryRules.payloadHash }).from(libraryRules).where(eq(libraryRules.ruleId, document.rule.id)).get();
  if (stored?.payloadHash === ruleDocumentDigest(document)) return;
  writeLibraryRule(tx, ruleCatalog, document, { now });
}

/**
 * Remove each Merge Assist rule attached to the repository that no checkout of it has on: its attachments and the
 * repository's and checkouts' settings for it, and the library rule itself once no repository has it attached. Runs
 * inside the caller's transaction; removing the library rule nests in it.
 */
function removeUnused(db: RerailDatabase, tx: StateQueries, repositoryKey: RepositoryKey): void {
  const paths = tx.select({ path: checkouts.checkoutPath }).from(checkouts).where(eq(checkouts.repositoryKey, repositoryKey)).all().map(row => row.path);
  const attached = new Set(tx.select({ id: ruleAttachments.sourceId }).from(ruleAttachments).where(and(
    eq(ruleAttachments.repositoryKey, repositoryKey),
    eq(ruleAttachments.sourceKind, "library_rule"),
    candidateRule(ruleAttachments.sourceId),
  )).all().map(row => row.id));
  for (const ruleId of attached) {
    const stillOn = paths.length > 0 && tx.select({ ownerId: ruleSettings.ownerId }).from(ruleSettings).where(and(
      eq(ruleSettings.ownerKind, "checkout"),
      inArray(ruleSettings.ownerId, paths),
      eq(ruleSettings.targetKind, "rule"),
      eq(ruleSettings.targetId, ruleId),
      eq(ruleSettings.on, true),
    )).get() !== undefined;
    if (stillOn) continue;
    tx.delete(ruleAttachments).where(and(
      eq(ruleAttachments.repositoryKey, repositoryKey),
      eq(ruleAttachments.sourceKind, "library_rule"),
      eq(ruleAttachments.sourceId, ruleId),
    )).run();
    tx.delete(ruleSettings).where(and(
      eq(ruleSettings.ownerKind, "repository"),
      eq(ruleSettings.ownerId, repositoryKey),
      eq(ruleSettings.targetKind, "rule"),
      eq(ruleSettings.targetId, ruleId),
    )).run();
    if (paths.length > 0) {
      tx.delete(ruleSettings).where(and(
        eq(ruleSettings.ownerKind, "checkout"),
        inArray(ruleSettings.ownerId, paths),
        eq(ruleSettings.targetKind, "rule"),
        eq(ruleSettings.targetId, ruleId),
      )).run();
    }
    const elsewhere = tx.select({ id: ruleAttachments.sourceId }).from(ruleAttachments).where(and(
      eq(ruleAttachments.sourceKind, "library_rule"),
      eq(ruleAttachments.sourceId, ruleId),
    )).get();
    if (elsewhere === undefined) removeLibraryRule(db, ruleId);
  }
}

/**
 * Merge Assist turned off for the repository: every rule it applied in any of its checkouts is removed, and so are the
 * findings from other branches, the in-flight conflicts (their dismissals included) and each checkout's scan and
 * activation state. The checkouts' own candidate and ledger entry files, and the record of where each came from, stay.
 */
export function deactivateRepository(db: RerailDatabase, repositoryKey: RepositoryKey): void {
  db.transaction(tx => {
    const paths = tx.select({ path: checkouts.checkoutPath }).from(checkouts).where(eq(checkouts.repositoryKey, repositoryKey)).all().map(row => row.path);
    if (paths.length > 0) {
      tx.delete(ruleSettings).where(and(
        eq(ruleSettings.ownerKind, "checkout"),
        inArray(ruleSettings.ownerId, paths),
        eq(ruleSettings.targetKind, "rule"),
        candidateRule(ruleSettings.targetId),
      )).run();
    }
    removeUnused(db, tx, repositoryKey);
    tx.delete(mergeAssistItems).where(and(eq(mergeAssistItems.repositoryKey, repositoryKey), eq(mergeAssistItems.origin, "peer"))).run();
    for (const path of paths) {
      deleteState(tx, STATE_KEYS.activation(path));
      deleteState(tx, STATE_KEYS.scan(path));
      deleteState(tx, STATE_KEYS.conflicts(path));
    }
  }, { behavior: "immediate" });
}
