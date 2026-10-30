/**
 * What the dashboard shows of rules waiting for a person's approval: the Waiting for approval page's entries
 * (`GET /api/rule-approvals`), how many wait, what a rule row says of a pending edit, and which rules the repository
 * holds that run nowhere until approved (`unavailable` with reason "awaiting-approval"). Pure: no DOM.
 */
import { ruleTitle } from "./derive.js";
import { fmt, plural } from "./format.js";
import { escapeInvisible, lineDiff, visibleJson } from "./rule-diff.js";

/** The hash of the Waiting for approval page (`routes.js`). */
export const APPROVALS_HASH = "approvals";

/** How many rules wait for approval in `approvals` (the list, `null` before it is read). */
export const awaitingCount = approvals => approvals?.length ?? 0;

/** "1 rule waits for approval", "3 rules wait for approval". */
export const awaitingText = n => `${fmt(n)} ${plural(n, "rule waits", "rules wait")} for approval`;

/** Who a waiting rule belongs to: a rule created by you, or a rule pack added from a folder or tarball. */
export const ownerText = (pending, packName = id => id) =>
  pending.owner === "library" ? "Created by you" : `Rule pack ${packName(pending.packId ?? "")}`;

/** The sentence a waiting rule leads with, as a rule row's title reads, invisible characters escaped. */
export const pendingTitle = pending => escapeInvisible(ruleTitle({ kind: pending.kind, rule: pending.rule }));

/** The title of a rule waiting here: from its waiting content, else `fallback(id)` (a pack's own title), else its id. */
export function waitingTitle(reference, approvals, fallback = () => undefined) {
  const owner = reference.sourceKind === "library_rule" ? "library" : "pack";
  const pending = (approvals ?? []).find(p => p.owner === owner && p.id === reference.id);
  return pending === undefined ? fallback(reference.id) ?? reference.id : pendingTitle(pending);
}

/**
 * One waiting rule as the page shows it: its title, owner, whether its approved content runs meanwhile (`runs`
 * "approved") or it runs nowhere yet ("nowhere"), its whole content as lines (`lines`, each `{ op, text }`; against
 * the approved content when there is one, so every line is shown and the changed ones are marked), and the body
 * Approve posts, naming the content read.
 */
export function approvalView(pending, packName) {
  const approved = pending.approved;
  const proposed = visibleJson(pending.rule);
  const lines = approved === null
    ? proposed.split("\n").map(text => ({ op: "same", text }))
    : lineDiff(visibleJson(approved.rule), proposed);
  return {
    key: `${pending.owner}:${pending.id}`,
    id: pending.id,
    kind: pending.kind,
    title: pendingTitle(pending),
    owner: ownerText(pending, packName),
    runs: approved === null ? "nowhere" : "approved",
    approvedAt: approved?.approvedAt ?? null,
    copiedFrom: pending.copiedFromRuleId,
    lines,
    changed: lines.filter(line => line.op !== "same").length,
    body: { owner: pending.owner, id: pending.id, contentHash: pending.contentHash },
  };
}

/**
 * What a rule row says of its `approval` (`{ state, approvedAt }`): nothing unless content waits; else whether the
 * approved content runs meanwhile (`runs` "approved") or the rule runs nowhere yet ("nowhere").
 */
export function pendingNote(approval) {
  if (approval?.state !== "pending") return null;
  return approval.approvedAt === null
    ? { runs: "nowhere", label: "Waits for approval", tip: "Runs nowhere until you approve it." }
    : { runs: "approved", label: "Edit waits for approval", tip: "The content you approved runs until you approve the edit." };
}

/**
 * The rules the repository holds that run nowhere until approved, once each however many folders add them:
 * `[{ sourceKind, id, folders }]`, from `unavailable` references with reason "awaiting-approval".
 */
export function awaitingReferences(unavailable) {
  const byRule = new Map();
  for (const reference of unavailable ?? []) {
    if (reference.reason !== "awaiting-approval") continue;
    const key = `${reference.sourceKind}:${reference.sourceId}`;
    const known = byRule.get(key);
    if (known === undefined) byRule.set(key, { sourceKind: reference.sourceKind, id: reference.sourceId, folders: [reference.folder] });
    else if (!known.folders.includes(reference.folder)) known.folders.push(reference.folder);
  }
  return [...byRule.values()];
}

/**
 * The waiting rules a pack page lists (`key` as `pageKey` in `pack-model.js` names it): a rule created by you on
 * Created by you ("mine"), a pack's rule on its pack's page. The pack comes from the waiting list, else `packIdOf(id)`.
 */
export function awaitingOnPage(references, key, approvals, packIdOf = () => undefined) {
  return references.filter(reference => {
    if (reference.sourceKind === "library_rule") return key === "mine";
    const listed = (approvals ?? []).find(p => p.owner === "pack" && p.id === reference.id);
    const packId = listed?.packId ?? packIdOf(reference.id);
    return packId !== undefined && packId !== null && key === `pack:${packId}`;
  });
}

/** Whether a failed Approve was refused because the rule changed after the page read it (409 `rule-changed`). */
export const isRuleChanged = error => /answered 409: .*"rule-changed"/su.test(String(error?.message ?? ""));
