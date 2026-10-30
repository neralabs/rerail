/**
 * The rule sheet's own state and clicks: the "Edit with coding agent" box, the "What the agent sees" hand-off and the
 * two-step Delete. At most one of the three is open; opening one closes the others.
 */
import { data, ui, panelRuleId, titleOf } from "../state.js";
import { modeLabel, scopeText } from "../derive.js";
import { copyText, invalidate, renderNow, toast } from "../shell.js";
import { removeRule } from "../rule-remove.js";
import { askPrompt, agentSeesPrompt } from "./rule-sheet-model.js";

/** The sheet's open boxes, for rule `ruleId`; a different rule starts closed (`syncSheetState`). */
export const sheetState = { ruleId: null, agentEdit: false, delConfirm: false };

/** Close the hand-off and Delete boxes when the sheet shows another rule than they were opened for. */
export function syncSheetState(id) {
  if (sheetState.ruleId === id) return;
  sheetState.ruleId = id;
  sheetState.agentEdit = false;
  sheetState.delConfirm = false;
}

/** What the prompts say about rule `stored`. */
export const askRule = stored => ({ id: stored.rule.id, title: titleOf(stored), scope: scopeText(stored), mode: modeLabel(stored) });

const openRuleStored = () => {
  const id = panelRuleId();
  return id === null ? undefined : data.rules.get(id);
};

const scrollSheetTo = selector => document.querySelector(`#panel ${selector}`)?.scrollIntoView({ block: "nearest" });

export const ruleSheetActions = {
  asktoggle: () => {
    ui.askOpen = !ui.askOpen;
    if (ui.askOpen) { sheetState.agentEdit = false; sheetState.delConfirm = false; }
    renderNow();
    if (ui.askOpen) scrollSheetTo(".sask");
  },
  askintent: intent => { ui.askIntent = intent; invalidate(); },
  askcopy: () => {
    const stored = openRuleStored();
    if (stored) copyText(askPrompt(askRule(stored), ui.askIntent), document.getElementById("askText"));
  },
  promptedit: () => {
    sheetState.agentEdit = true;
    sheetState.delConfirm = false;
    ui.askOpen = false;
    renderNow();
    document.getElementById("agentDraft")?.focus();
  },
  promptcancel: () => { sheetState.agentEdit = false; invalidate(); },
  promptcopy: () => {
    const draft = document.getElementById("agentDraft");
    const id = panelRuleId();
    if (id === null || !draft) return;
    if (draft.value.trim() === "") { draft.focus(); return; }
    copyText(agentSeesPrompt(id, draft.value), draft);
  },
  delask: () => {
    sheetState.delConfirm = !sheetState.delConfirm;
    sheetState.agentEdit = false;
    ui.askOpen = false;
    renderNow();
    scrollSheetTo(".pfoot");
  },
  delcancel: () => { sheetState.delConfirm = false; invalidate(); },
  delgo: id => { sheetState.delConfirm = false; invalidate(); return removeRule(id); },
  /** A mode button, read-only in this release: its title says why, and so does the toast. */
  modeinfo: (_, b) => toast(`${b.title}.`),
};
