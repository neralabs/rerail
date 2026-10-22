/**
 * One rule pack's page (`#pack/<packId>`) and Created by you (`#created`): that section of the rule list, with its
 * groups, rows and switches, under the rule list's controls.
 */
import { h, setContent, icon } from "../dom.js";
import { data, ui, ruleSections, packById, repoName, summary } from "../state.js";
import { renderRules, ruleListActions } from "../render/rules.js";
import { ruleControls, ruleControlActions, ruleControlInput, ruleControlChange } from "../render/rule-controls.js";

export const needs = ["rules", "packs", "activity"];

/** The rule list section the page shows. */
const sectionKey = () => (ui.view === "created" ? "mine" : `pack:${ui.pack}`);

let page = null;
let notice = null;

function build() {
  const tools = h("div.pktools");
  const controls = h("div");
  const list = h("div");
  page = { root: h("div.pk", null, tools, controls, list), tools, controls, list };
  notice = h("div.pk");
}

function renderTools(tools) {
  const created = ui.view === "created";
  setContent(tools, `${ui.view}|${ui.pack}`, () => (created
    ? h("button.btn.sm", { "data-act": "nav", "data-i": "create" }, icon("plus"), "Create a rule")
    : h("button.btn.sm", { "data-act": "viewpack", "data-i": ui.pack }, "About this rule pack")));
}

/** The pack is not added here, not one Rerail knows, or the rules are still loading. */
function renderNotice(root) {
  if (notice.parentNode !== root) root.replaceChildren(notice);
  const pack = packById(ui.pack), repo = repoName() || "this repo";
  setContent(notice, JSON.stringify([data.loaded, ui.pack, pack?.name, repo]), () => h("div.list", null, h("div.empty", null,
    !data.loaded ? h("h3", null, "Loading…")
      : pack === undefined ? [h("h3", null, "Rerail has no rule pack by that name"), h("p", null, h("span.mono", null, ui.pack))]
        : [h("h3", null, `${pack.name} is not added to ${repo}`), h("p", null, pack.covers),
          h("p.acts", null, h("button.btn.sm.solid", { "data-act": "addpack", "data-i": pack.id }, icon("plus"), "Add pack"), " ",
            h("button.btn.sm", { "data-act": "viewpack", "data-i": pack.id }, "About this rule pack"))])));
}

export function render(root) {
  if (page === null) build();
  const key = sectionKey();
  if (!ruleSections().some(s => s.key === key)) { renderNotice(root); return; }
  if (page.root.parentNode !== root) root.replaceChildren(page.root);
  renderTools(page.tools);
  const controls = ruleControls(key);
  if (controls.parentNode !== page.controls) page.controls.replaceChildren(controls);
  renderRules(page.list, summary(), key);
}

export const actions = { ...ruleListActions, ...ruleControlActions };
export const onInput = ruleControlInput;
export const onChange = ruleControlChange;
