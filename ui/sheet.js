/**
 * The sheet (`#panel`): a rule's or a rule pack's detail beside the current view, from `render/panel.js` and
 * `render/pack-panel.js`. Its own actions, and what a poll loads while it is open.
 */
import { data, ui, panelRuleId, titleOf } from "./state.js";
import { packName } from "./attachments.js";
import { renderPanel, askPrompt } from "./render/panel.js";
import { closePanel, copyText, invalidate, openRule, ownerWords, writes } from "./shell.js";

/** An open sheet shows its rule's numbers and recent violations, and a pack's rules. */
export const SHEET_NEEDS = ["rules", "packs", "activity", "ruleFindings"];

/** Render whichever sheet `ui.panel` names; `body[data-panel]` makes room for it. */
export const renderSheet = renderPanel;

/** Honour or ignore linter suppression comments for one rule. */
function suppressRule(id, suppressions) {
  const stored = data.rules.get(id);
  if (!stored) return;
  writes.setSuppressions([id], suppressions, suppressions === "ignore"
    ? `Suppression comments ignored ${ownerWords()}. “${titleOf(stored)}” reports a violation on a line a linter comment disables.`
    : `Suppression comments honoured ${ownerWords()}. “${titleOf(stored)}” skips a line a linter comment disables.`);
}

/** Honour or ignore linter suppression comments for every rule of a pack. */
function suppressPack(id, suppressions) {
  writes.setPackSuppressions(id, suppressions, suppressions === "ignore"
    ? `Suppression comments ignored ${ownerWords()} in ${packName(id)}. A rule with its own setting keeps it.`
    : `Suppression comments honoured ${ownerWords()} in ${packName(id)}. A rule with its own setting keeps it.`);
}

export const sheetActions = {
  packrule: id => openRule(id, ui.panel?.kind === "pack" ? ui.panel.id : null),
  panelback: () => {
    if (!ui.backToPack) { closePanel(); return; }
    ui.panel = { kind: "pack", id: ui.backToPack };
    ui.backToPack = null;
    invalidate();
  },
  suppress: (id, b) => suppressRule(id, b.dataset.v),
  packsuppress: (id, b) => suppressPack(id, b.dataset.v),
  asktoggle: id => { ui.askOpen = !ui.askOpen; openRule(id, ui.backToPack); },
  askintent: id => { ui.askIntent = id; invalidate(); },
  askcopy: () => {
    const id = panelRuleId(), stored = id === null ? undefined : data.rules.get(id);
    if (stored) copyText(askPrompt(stored, ui.askIntent), document.getElementById("askText"));
  },
};
