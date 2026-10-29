/**
 * Actions any region of the page may carry (`data-act`): the ones that open the sheet, move to another view or
 * repository, or act on something more than one view shows (a rule's switch, a pack's Add and Remove, the time
 * range). A click looks up its region's own actions first (the current view's, the sheet's or the frame's), then
 * these. Each is `(id, element) => void`, `id` being the element's `data-i`.
 */
import { send } from "./api.js";
import { data, ui, titleOf, packOfRule, packById, rulesArray, reviewing } from "./state.js";
import { refreshNow } from "./poll.js";
import { failed } from "./format.js";
import { packName } from "./attachments.js";
import { parseRoute } from "./routes.js";
import { closePanel, copyText, go, invalidate, openPack, openRule, showRepository, toast, writes } from "./shell.js";
import { restoreRule } from "./rule-remove.js";

function toggleRule(id) {
  const stored = data.rules.get(id);
  if (!stored) return;
  const enabled = !stored.enabled, title = titleOf(stored), state = enabled ? "on" : "off";
  writes.setEnabled([id], enabled, enabled ? `${title} is on from the coding agent’s next turn.` : `${title} is off.`,
    `Couldn’t turn ${title} ${state}`);
}

/** What an added pack turns on: its rules, but not the opt-in ones the pack starts off. */
function addedRulesText(id) {
  const pack = packById(id);
  const off = pack ? pack.rules.filter(r => !r.on).length : 0;
  return off === 0 ? "Its rules are on." : `${off} of its rules start off; turn them on from its page.`;
}

/** After a switch in the review. */
const keptMessage = (id, keep) => `${packName(id)} ${keep ? "on" : "off"}.`;

/**
 * After a pack is removed: its rules that stay are the ones added to the repository on their own. No view lists them
 * yet (`ruleSections()` keeps them in a section nothing draws), so the toast names no place to find them.
 */
function removedMessage(id) {
  const kept = rulesArray().filter(r => packOfRule(r)?.id === id).length;
  return kept === 0 ? `${packName(id)} removed. Its rules no longer run here.`
    : `${packName(id)} removed. ${kept} of its ${kept === 1 ? "rules stays" : "rules stay"}, added on their own.`;
}

/** Keep or switch off pack `id` during the review of a repository not set up yet. */
export const keepPack = (id, keep) => writes.keepPack(id, keep, keptMessage(id, keep), `Couldn’t turn ${packName(id)} ${keep ? "on" : "off"}`);

/** Start or stop every checkout of a repository (people only; it clears each checkout's own value). */
async function setRepository(key, verb) {
  // Settings lists every repository, so the one acted on is named from that list, not from the repository shown.
  const name = data.repositories.find(r => r.repositoryKey === key)?.name ?? (data.repository?.repositoryKey === key ? data.repository.name : "this repo");
  try {
    const { body: { repository } } = await send("POST", `/api/repository/${verb}`, { repository: key, scope: "repository" });
    toast(verb === "stop" ? `Rerail stopped in ${repository.name}. Your rules and history are kept.`
      : repository.setUp ? `Rerail started in ${repository.name}.`
        : `Rerail started in ${repository.name}. Add rule packs to run rules here.`);
  } catch (error) {
    toast(failed(`Couldn’t ${verb} Rerail in ${name}`, error));
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
  addpack: id => (reviewing() ? keepPack(id, true) : writes.addPacks([id], `${packName(id)} added. ${addedRulesText(id)}`, `Couldn’t add ${packName(id)}`)),
  removepack: id => {
    if (ui.removeAsk !== id) { ui.removeAsk = id; invalidate(); return; }
    if (reviewing()) keepPack(id, false);
    else writes.removePack(id, () => removedMessage(id), `Couldn’t remove ${packName(id)}`);
  },
  cancelremove: () => { ui.removeAsk = null; invalidate(); },
  session: id => { closePanel(); go({ view: "sessions", session: id, jump: true }); },
  actrule: id => go({ view: "history", filterRule: id }),
  range: id => { ui.range = Number(id); invalidate(); refreshNow(); },
  cmdcopy: text => copyText(text, null, `Copied ${text}.`),
  pickrepo: key => showRepository(key),
  repostart: key => setRepository(key, "start"),
  repostop: key => setRepository(key, "stop"),
  /** The Undo on a deleted rule's toast. */
  ruleundo: id => restoreRule(id),
};
