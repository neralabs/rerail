/**
 * A rule a coding agent edited: the edit is saved as a copy (`copiedFromRuleId`) that waits for approval while the
 * original keeps running. The copy's sheet asks for the approval, saying which rule it edits and which parts changed;
 * the original's sheet says an edit of it waits, with a button to the copy, and its row in a rule table carries
 * "Edited". Both last until the edit is approved or discarded: neither can be dismissed, as nothing that asks for an
 * approval can.
 */
import { awaitingApproval, isLog, isWaitingEdit, messageText } from "../derive.js";
import { data } from "../state.js";
import { whenLabel } from "../format.js";

/** The waiting copies by the id of the rule each was copied from, worked out once per rules list. */
const copiesByRules = new WeakMap();

function waitingCopies(rules) {
  let copies = copiesByRules.get(rules);
  if (copies === undefined) {
    copies = new Map();
    for (const stored of rules.values()) if (isWaitingEdit(stored)) copies.set(stored.copiedFromRuleId, stored);
    copiesByRules.set(rules, copies);
  }
  return copies;
}

/**
 * The copy of `stored` that waits for approval, else `null`. Also `null` while `stored` itself waits: its sheet then
 * shows that approval instead, so its row says nothing of the edit either.
 */
export function pendingEditOf(stored) {
  if (awaitingApproval(stored)) return null;
  return waitingCopies(data.rules).get(stored.rule.id) ?? null;
}

const json = value => JSON.stringify(value ?? null);
/** A rule's clauses without their violation message, so a change of what it says is not read as a change of what it checks. */
const withoutMessage = clauses => (clauses ?? []).map(clause => ({ ...clause, violation: { ...clause.violation, message: undefined } }));

/**
 * Whether a rule is set to Log only. A rule that waits runs in Log whatever it is set to; one set to steer or block
 * says so in `modeSource` (`awaiting-approval`), which approving lifts.
 */
const setToLog = stored => isLog(stored) && stored.modeSource?.by !== "awaiting-approval";

/**
 * The sheet's sections whose content the edit changed, in the sheet's order and by its own names. "On a violation" is
 * the rule's mode: a coding agent changes it through the rule's document, so Log to Steer is an edit like any other and
 * waits for the same approval.
 */
export function changedParts(was, now) {
  const a = was.rule, b = now.rule;
  return [
    setToLog(was) !== setToLog(now) && "On a violation",
    messageText(was) !== messageText(now) && "What the agent sees",
    (a.description ?? "") !== (b.description ?? "") && "Description",
    json(a.scope) !== json(b.scope) && "Where it applies",
    json(withoutMessage(a.clauses)) !== json(withoutMessage(b.clauses)) && "What it checks",
  ].filter(Boolean);
}

/** The notice's title: the edit is not live until a person approves the copy. */
export const EDIT_TITLE = "An edit of this rule waits for approval.";

/** The notice's first line: when the edit was saved (the copy's `addedAt`), and that the rule runs as it was meanwhile. */
export const editedLine = copy => `${copy.addedAt === null ? "" : `Edited ${whenLabel(copy.addedAt)}. `}This rule keeps running as it was until the edit is approved.`;

/** The hover of a row's "Edited". */
export const EDITED_TIP = "An edit of this rule waits for approval. Open it to review the edit.";
