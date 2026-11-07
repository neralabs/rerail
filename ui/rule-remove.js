/**
 * Deleting a rule created by you from this repository, from the rule sheet: one `PUT /api/repository/attachments`
 * that removes its library-rule attachments, queued among the attachment writes in click order. The rule stays in
 * the library (no `DELETE /api/library-rules`, which would take it out of every repository). The toast's Undo
 * (`ruleundo`) adds the same attachments back.
 */
import { data } from "./state.js";
import { captureScope, queueInScope, refreshNow } from "./poll.js";
import { closePanel, toast } from "./shell.js";
import { libraryAttachments } from "./render/rule-sheet-model.js";

const ATTACHMENTS = "/api/repository/attachments";

/** Rule id → the attachments its delete removed, while its Undo can still add them back. */
const removed = new Map();

/** Send one attachment write; `true` once it is answered OK and the page still shows the scope. */
async function write(body, failure) {
  try {
    return await queueInScope(captureScope(), "PUT", ATTACHMENTS, body);
  } catch (error) {
    toast(`${failure}: ${error.message}`);
    return false;
  }
}

/** Remove rule `id` from this repository; the sheet closes and the toast offers Undo. */
export async function removeRule(id) {
  const stored = data.rules.get(id);
  if (!stored) return;
  const attachments = libraryAttachments(stored);
  if (attachments.length === 0) return;
  if (!(await write({ remove: attachments }, `Could not delete “${id}”`))) return;
  removed.set(id, attachments);
  closePanel();
  await refreshNow();
  toast(`Deleted “${id}” from this repository.`, { action: { label: "Undo", act: "ruleundo", id } });
}

/** Add rule `id` back where its delete removed it. */
export async function restoreRule(id) {
  const attachments = removed.get(id);
  if (!attachments) return;
  if (!(await write({ add: attachments }, `Could not restore “${id}”`))) return;
  removed.delete(id);
  await refreshNow();
  toast(`Restored “${id}”.`);
}
