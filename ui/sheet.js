/**
 * The sheet (`#panel`): a rule's or a rule pack's detail beside the current view, from `render/panel.js` and
 * `render/pack-panel.js`. Its own actions (the rule sheet's own are `ruleSheetActions`), and what a poll loads while
 * it is open.
 */
import { ui } from "./state.js";
import { packName } from "./attachments.js";
import { renderPanel } from "./render/panel.js";
import { ruleSheetActions } from "./render/rule-sheet-actions.js";
import { closePanel, invalidate, openRule, writes } from "./shell.js";

/** An open sheet shows its rule's numbers and recent violations, and a pack's rules. */
export const SHEET_NEEDS = ["rules", "packs", "activity", "ruleFindings"];

/** Render whichever sheet `ui.panel` names; `body[data-panel]` makes room for it. */
export const renderSheet = renderPanel;

/** Honour or ignore linter suppression comments for every rule of a pack. */
function suppressPack(id, suppressions) {
  const verb = suppressions === "ignore" ? "ignores" : "honours";
  writes.setPackSuppressions(id, suppressions, `${packName(id)} now ${verb} suppression comments. Rules with their own setting keep it.`,
    `Couldn’t change suppression comments for ${packName(id)}`);
}

export const sheetActions = {
  packrule: id => openRule(id, ui.panel?.kind === "pack" ? ui.panel.id : null),
  panelback: () => {
    if (!ui.backToPack) { closePanel(); return; }
    ui.panel = { kind: "pack", id: ui.backToPack };
    ui.backToPack = null;
    invalidate();
  },
  packsuppress: (id, b) => suppressPack(id, b.dataset.v),
  ...ruleSheetActions,
};
