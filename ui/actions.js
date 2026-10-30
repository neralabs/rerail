/**
 * Actions any region of the page may carry (`data-act`): the ones that open the sheet, move to another view or
 * repository, or act on something more than one view shows (a rule's switch, a pack's Add and Remove, the time
 * range). A click looks up its region's own actions first (the current view's, the sheet's or the frame's), then
 * these. Each is `(id, element) => void`, `id` being the element's `data-i`.
 */
import { send } from "./api.js";
import { data, ui, titleOf, packOfRule, packById, rulesArray, reviewing } from "./state.js";
import { refreshNow } from "./poll.js";
import { packName } from "./attachments.js";
import { parseRoute } from "./routes.js";
import { closePanel, copyText, go, invalidate, openPack, openRule, showRepository, toast, writes } from "./shell.js";
import { restoreRule } from "./rule-remove.js";

function toggleRule(id) {
  const stored = data.rules.get(id);
  if (!stored) return;
  const enabled = !stored.enabled;
  writes.setEnabled([id], enabled, enabled
    ? `On. Rerail audits “${titleOf(stored)}” from the coding agent’s next turn.`
    : `Off. Rerail stops auditing “${titleOf(stored)}”.`);
}

/** What an added pack turns on: its rules, but not the opt-in ones the pack starts off. */
function addedRulesText(id) {
  const pack = packById(id);
  const off = pack ? pack.rules.filter(r => !r.on).length : 0;
  return off === 0 ? "Its rules are on." : `Its rules are on, except ${off} that start off.`;
}

/** After a switch in the review. */
const keptMessage = (id, keep) => `${packName(id)} ${keep ? "kept on" : "switched off"}.`;

/** After a pack is removed: its rules that stay are the ones added to the repository on their own. */
function removedMessage(id) {
  const kept = rulesArray().filter(r => packOfRule(r)?.id === id).length;
  return kept === 0 ? `${packName(id)} removed. Its rules no longer run here.`
    : `${packName(id)} removed. ${kept} of its ${kept === 1 ? "rules stays" : "rules stay"}, added on their own, on Overview under Other rules by Rerail.`;
}

/** Keep or switch off pack `id` during the review of a repository not set up yet. */
export const keepPack = (id, keep) => writes.keepPack(id, keep, keptMessage(id, keep));

/** Start or stop every checkout of a repository (people only; it clears each checkout's own value). */
async function setRepository(key, verb) {
  try {
    const { body } = await send("POST", `/api/repository/${verb}`, { repository: key, scope: "repository" });
    toast(verb === "start" ? `Rerail started in ${body.repository.name}.` : `Rerail stopped in ${body.repository.name}. Rules and history are kept.`);
  } catch (error) {
    toast(`Could not ${verb} Rerail: ${error.message}`);
  }
  await refreshNow();
}

/**
 * Approve `subject` (the rule by its id, or the pack by package name, at the digest this page read), for this worktree
 * when the page names one, where its own rules are read. Content that changed since is refused, and the page reloads.
 */
async function approveContent(subject) {
  const checkout = ui.scope?.checkout;
  try {
    await send("POST", "/api/rule-approvals", { ...subject, ...(checkout === undefined ? {} : { checkout }) });
    toast(`Approved ${subject.id}. It runs in the mode it is set to from the coding agent’s next turn.`);
  } catch (error) {
    toast(`Could not approve: ${error.message}`);
  }
  await refreshNow();
}

export const commonActions = {
  /** `data-i` is a view's hash (`routes.js`): "overview", "pack/<id>", "packs", "create", "history"… */
  nav: hash => { const route = parseRoute(hash); if (route.view !== null) go(route); },
  detail: id => openRule(id),
  viewpack: id => openPack(id),
  close: () => closePanel(),
  toggle: id => toggleRule(id),
  // During the review of a repository not set up yet, Add and Remove are the review's switch.
  addpack: id => (reviewing() ? keepPack(id, true) : writes.addPacks([id], `${packName(id)} added. ${addedRulesText(id)}`)),
  removepack: id => {
    if (ui.removeAsk !== id) { ui.removeAsk = id; invalidate(); return; }
    if (reviewing()) { keepPack(id, false); return; }
    // Removed from the catalog: its row reads Removed, with Undo, until the page is left.
    if (ui.view === "packs") ui.removedPacks.add(id);
    writes.removePack(id, () => removedMessage(id));
  },
  cancelremove: () => { ui.removeAsk = null; invalidate(); },
  session: id => { closePanel(); go({ view: "sessions", session: id, jump: true }); },
  actrule: id => go({ view: "history", filterRule: id }),
  range: id => { ui.range = Number(id); invalidate(); refreshNow(); },
  cmdcopy: text => copyText(text, null, `Copied ${text}`),
  pickrepo: key => showRepository(key),
  repostart: key => setRepository(key, "start"),
  repostop: key => setRepository(key, "stop"),
  /** The Undo on a deleted rule's toast. */
  ruleundo: id => restoreRule(id),
  /** Approve a rule's or pack's content as this page read it (`data-kind`, `data-digest`): it then runs in its mode. */
  approve: (id, element) => approveContent({ kind: element.dataset.kind, id, digest: element.dataset.digest }),
};
