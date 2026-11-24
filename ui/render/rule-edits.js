/**
 * A rule a coding agent edited: the edit is saved as a copy (`copiedFromRuleId`) that waits for approval while the
 * original keeps running. The original's sheet says so once, which parts changed and how to undo it, and its row in a
 * rule table carries "Edited", until the person dismisses the notice. Dismissing is per edit (the copy's approval
 * digest) and lasts on this computer, as Overview's banner for a newer Rerail does.
 */
import { awaitingApproval, isEditedCopy, messageText } from "../derive.js";
import { data } from "../state.js";
import { whenLabel } from "../format.js";

/** The waiting copies by the id of the rule each was copied from, worked out once per rules list. */
const copiesByRules = new WeakMap();

function waitingCopies(rules) {
  let copies = copiesByRules.get(rules);
  if (copies === undefined) {
    copies = new Map();
    for (const stored of rules.values()) if (isEditedCopy(stored) && awaitingApproval(stored)) copies.set(stored.copiedFromRuleId, stored);
    copiesByRules.set(rules, copies);
  }
  return copies;
}

const dismissKey = digest => `rerail-edit-dismissed:${digest}`;
/** The edits dismissed in this visit, which holds when storage is blocked. */
const dismissedHere = new Set();

export function editDismissed(digest) {
  if (dismissedHere.has(digest)) return true;
  try { return localStorage.getItem(dismissKey(digest)) === "1"; } catch { return false; /* storage blocked */ }
}

/** The notice's ✕: the edit whose copy has approval digest `digest` is not shown again on this computer. */
export function dismissEdit(digest) {
  dismissedHere.add(digest);
  try { localStorage.setItem(dismissKey(digest), "1"); } catch { /* private mode: dismissed for this visit */ }
}

/**
 * The copy of `stored` that waits for approval and whose notice was not dismissed, else `null`. Also `null` while
 * `stored` itself waits: its sheet then shows that approval instead, so its row says nothing of the edit either.
 */
export function pendingEditOf(stored) {
  if (awaitingApproval(stored)) return null;
  const copy = waitingCopies(data.rules).get(stored.rule.id);
  return copy === undefined || editDismissed(copy.approval.subject.digest) ? null : copy;
}

const json = value => JSON.stringify(value ?? null);
/** A rule's clauses without their violation message, so a change of what it says is not read as a change of what it checks. */
const withoutMessage = clauses => (clauses ?? []).map(clause => ({ ...clause, violation: { ...clause.violation, message: undefined } }));

/**
 * The sheet's sections whose content the edit changed, in the sheet's order and by its own names. Not the mode: Log or
 * Steer is a setting, changed without a copy.
 */
export function changedParts(was, now) {
  const a = was.rule, b = now.rule;
  return [
    messageText(was) !== messageText(now) && "What the agent sees",
    (a.description ?? "") !== (b.description ?? "") && "Description",
    json(a.scope) !== json(b.scope) && "Where it applies",
    json(withoutMessage(a.clauses)) !== json(withoutMessage(b.clauses)) && "What it checks",
  ].filter(Boolean);
}

/** The notice's first line: when the edit was saved, from the copy's `addedAt`. */
export const editedTitle = copy => (copy.addedAt === null ? "This rule was edited." : `This rule was edited ${whenLabel(copy.addedAt)}.`);

/** What a person pastes into a session with their coding agent to undo the edit of rule `id`. */
export const undoEditPrompt = id => `/rerail-create undo the edit to ${id}`;

/** The hover of a row's "Edited". */
export const EDITED_TIP = "This rule was edited. Open it to see what changed.";
