/**
 * Deleting a rule created by you from this repository, from the rule sheet: one `PUT /api/repository/attachments`
 * that removes its library-rule attachments, queued among the attachment writes in click order. The rule stays in
 * the library (no `DELETE /api/library-rules`, which would take it out of every repository). The toast's Undo
 * (`ruleundo`) adds the same attachments back.
 */
import { data, repoName, titleOf } from "./state.js";
import { failed as failedText } from "./format.js";
import { send } from "./api.js";
import { captureScope, inScope, queueInScope, refreshNow } from "./poll.js";
import { closePanel, toast } from "./shell.js";
import { libraryAttachments } from "./render/rule-sheet-model.js";

const ATTACHMENTS = "/api/repository/attachments";

/** Rule id → its title, the attachments its delete removed and where they were, while its Undo can still add them back. */
const removed = new Map();

/** Send one attachment write; `true` once it is answered OK and the page still shows the scope. */
async function write(scope, body, failure) {
  try {
    return await queueInScope(scope, "PUT", ATTACHMENTS, body);
  } catch (error) {
    toast(failedText(failure, error));
    return false;
  }
}

/** Remove rule `id` from this repository; the sheet closes and the toast offers Undo. */
export async function removeRule(id) {
  const stored = data.rules.get(id);
  if (!stored) return;
  const attachments = libraryAttachments(stored), title = titleOf(stored);
  if (attachments.length === 0) return;
  // Undo adds it back here, also once the page shows another repository.
  const scope = captureScope(), repo = repoName() || "this repo";
  if (!(await write(scope, { remove: attachments }, `Couldn’t delete ${title}`))) return;
  removed.set(id, { title, attachments, scope, repo });
  closePanel();
  await refreshNow();
  toast(`Deleted from ${repo}.`, { action: { label: "Undo", act: "ruleundo", id } });
}

/**
 * Add rule `id` back where its delete removed it. While the page still shows that repository the write queues among
 * its attachment writes; once it shows another, that queue is the other repository's, so the write goes on its own.
 */
export async function restoreRule(id) {
  const entry = removed.get(id);
  if (!entry) return;
  const body = { add: entry.attachments }, failure = `Couldn’t restore ${entry.title}`;
  if (inScope(entry.scope)) {
    if (!(await write(entry.scope, body, failure))) return;
  } else {
    try {
      await send("PUT", ATTACHMENTS, { ...entry.scope.target, ...body });
    } catch (error) {
      toast(failedText(failure, error));
      return;
    }
  }
  removed.delete(id);
  await refreshNow();
  toast(`Restored to ${entry.repo}.`);
}
