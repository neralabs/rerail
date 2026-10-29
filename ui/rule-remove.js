/**
 * Deleting a rule created by you from this repository, from the rule sheet: one `PUT /api/repository/attachments`
 * that removes its library-rule attachments, queued among the attachment writes in click order. The rule stays in
 * the library (no `DELETE /api/library-rules`, which would take it out of every repository). The toast's Undo
 * (`ruleundo`) adds the same attachments back.
 */
import { data, repoName, titleOf } from "./state.js";
import { failed as failedText } from "./format.js";
import { captureScope, queueInScope, refreshNow } from "./poll.js";
import { closePanel, toast } from "./shell.js";
import { libraryAttachments } from "./render/rule-sheet-model.js";

const ATTACHMENTS = "/api/repository/attachments";

/** Rule id → its title and the attachments its delete removed, while its Undo can still add them back. */
const removed = new Map();

/** Send one attachment write; `true` once it is answered OK and the page still shows the scope. */
async function write(body, failure) {
  try {
    return await queueInScope(captureScope(), "PUT", ATTACHMENTS, body);
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
  if (!(await write({ remove: attachments }, `Couldn’t delete ${title}`))) return;
  removed.set(id, { title, attachments });
  closePanel();
  await refreshNow();
  toast(`${title} deleted from ${repoName() || "this repo"}.`, { action: { label: "Undo", act: "ruleundo", id } });
}

/** Add rule `id` back where its delete removed it. */
export async function restoreRule(id) {
  const entry = removed.get(id);
  if (!entry) return;
  if (!(await write({ add: entry.attachments }, `Couldn’t restore ${entry.title}`))) return;
  removed.delete(id);
  await refreshNow();
  toast(`${entry.title} restored.`);
}
