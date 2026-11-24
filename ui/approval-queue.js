/**
 * What a coding agent wrote that waits for a person's approval, as one queue: the rules it added and the edits it made
 * (`derive.js` `awaitingAgentApproval`). Overview's banner opens the first, and a rule's sheet says where it stands
 * ("2 of 4 waiting") and, once it is approved, offers the next. The one that caught the most comes first, then by title.
 *
 * Also which rules this page showed waiting, so a sheet can say "Approved." for one approved in this visit. That is page
 * state only: a reload forgets it.
 */
import { awaitingAgentApproval } from "./derive.js";
import { data, allFires, rulesArray, titleOf } from "./state.js";

/** The rules a coding agent added or edited that wait for approval, in the order they are reviewed. */
export const approvalQueue = () => (data.loaded ? rulesArray().filter(awaitingAgentApproval) : [])
  .sort((a, b) => allFires(b.rule.id) - allFires(a.rule.id)
    || titleOf(a).localeCompare(titleOf(b), undefined, { sensitivity: "base" }) || a.rule.id.localeCompare(b.rule.id));

/** Where rule `id` stands among `queue`, as its sheet says it ("2 of 4 waiting"); `null` alone in it or not in it. */
export function queuePosition(queue, id) {
  const at = queue.findIndex(stored => stored.rule.id === id);
  return queue.length > 1 && at >= 0 ? `${at + 1} of ${queue.length} waiting` : null;
}

/** The ids of the rules a sheet showed waiting in this visit, less the ones whose "Approved." was dismissed. */
const shownWaiting = new Set();

/** A sheet shows rule `id` waiting. */
export const noteWaiting = id => { shownWaiting.add(id); };
/** Whether rule `id` was shown waiting in this visit: no longer waiting, it was approved here. */
export const wasShownWaiting = id => shownWaiting.has(id);
/** The ✕ on a sheet's "Approved.": it is not shown for rule `id` again. */
export const dismissApproved = id => { shownWaiting.delete(id); };
