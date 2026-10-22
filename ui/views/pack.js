/**
 * One rule pack's page (`#pack/<packId>`) and Created by you (`#created`). The heading leads: the name, how many of its
 * rules are on (or shown, under a search or filter), its share of the violations logged, what the pack covers and a
 * quiet Remove pack. Then Search, Filter and Sort with the whole-pack switch at the right end, on the On/off column,
 * and the rule table. No primary button: the switches are the page's action. A page with nothing to list keeps its own.
 */
import { h, setAttr, setClass, setContent, setText, icon } from "../dom.js";
import { fmt } from "../format.js";
import { data, ui, packById, repoName, summary, rangeLabel, fires, filtering, isAttached } from "../state.js";
import { ownerWords, writes } from "../shell.js";
import { pageKey, pageRules, packSwitch, runningOf, shownRules, switchTurnsOn } from "../pack-model.js";
import { CREATE_COMMAND, renderRuleTable } from "../render/rules.js";
import { ruleControls, ruleControlActions, ruleControlInput, ruleControlChange } from "../render/rule-controls.js";
import { removePackControl } from "../render/pack-panel.js";

export const needs = ["rules", "packs", "activity"];

const CREATED = "Created by you";

let page = null;
let notice = null;

function build() {
  const title = h("h2"), count = h("span.n2"), share = h("span.shr"), remove = h("span.rmslot"), pitch = h("p.pitch");
  const head = h("div.ghead.pk", null, title, h("p.sub", null, count, share, remove), pitch);
  const packAllText = h("span.t");
  const packAll = h("div.packall", null, packAllText, h("button.sw", { role: "switch", "data-act": "ptoggle" }));
  const controls = h("div");
  const table = h("div");
  page = { root: h("div.ppage", null, head, controls, table), title, count, share, remove, pitch, packAll, packAllText, controls, table };
  notice = h("div.ppage");
}

/** The pack is not added here, not one Rerail knows, nothing is created yet, or the rules are still loading. */
function renderNotice(root) {
  if (notice.parentNode !== root) root.replaceChildren(notice);
  const created = ui.view === "created", pack = created ? null : packById(ui.pack), repo = repoName() || "this repo";
  const added = pack ? isAttached(pack) : false;
  setContent(notice, JSON.stringify([data.loaded, ui.view, ui.pack, pack?.name, added, ui.removeAsk, repo]), () => h("div.list", null, h("div.empty", null,
    !data.loaded ? h("h3", null, "Loading…")
      : created ? [h("h3", null, "No rules of your own yet"),
        h("p", null, "Write one with your coding agent: run ", h("span.mono", null, CREATE_COMMAND), " in a session and it lands here."),
        h("p.acts", null, h("button.btn.sm.solid", { "data-act": "nav", "data-i": "create" }, icon("plus"), "Create a rule"))]
      : pack === undefined ? [h("h3", null, "Rerail has no rule pack by that name"), h("p", null, h("span.mono", null, ui.pack))]
        : added ? [h("h3", null, `${pack.name} has no rules in ${repo} yet`), h("p", null, pack.covers), h("p.acts", null, removePackControl(pack))]
        : [h("h3", null, `${pack.name} is not added to ${repo}`), h("p", null, pack.covers),
          h("p.acts", null, h("button.btn.sm.solid", { "data-act": "addpack", "data-i": pack.id }, icon("plus"), "Add pack"))])));
}

function renderHead(all, shown, pack) {
  const name = pack?.name ?? CREATED, onN = runningOf(all);
  setText(page.title, name);
  setText(page.count, filtering() && shown.length < all.length ? `${fmt(shown.length)} of ${fmt(all.length)} shown` : `${fmt(onN)} of ${fmt(all.length)} on`);
  setClass(page.count, "mix", onN < all.length);
  const total = summary().triggers, n = all.reduce((sum, r) => sum + fires(r.rule.id), 0);
  setAttr(page.share, "hidden", total === 0);
  setText(page.share, `${fmt(n)} of ${fmt(total)} violations logged`);
  setAttr(page.share, "title", `Violations logged by these rules, of all violations ${rangeLabel()}`);
  // The one Remove on a pack page is the red pill; the catalog's list keeps the quiet link.
  setContent(page.remove, JSON.stringify([pack?.id, ui.removeAsk === pack?.id, repoName()]), () => (pack ? removePackControl(pack) : null));
  setAttr(page.pitch, "hidden", !pack);
  setText(page.pitch, pack?.covers ?? "");
}

/** The whole-pack switch: "x of y <pack> rules on" and drawn mixed while some are off; "All y <pack> rules" otherwise. */
function renderPackAll(all, name) {
  const { on, total, state } = packSwitch(all);
  setText(page.packAllText, state === "mixed" ? `${fmt(on)} of ${fmt(total)} ${name} rules on` : `All ${fmt(total)} ${name} rules`);
  const sw = page.packAll.lastChild, verb = switchTurnsOn(state) ? "on" : "off";
  setAttr(sw, "aria-checked", state);
  setClass(sw, "mixed", state === "mixed");
  setAttr(sw, "aria-label", `Turn all ${fmt(total)} ${name} rules ${verb}`);
  setAttr(sw, "title", `Turn all ${fmt(total)} rules ${verb}`);
}

export function render(root) {
  if (page === null) build();
  const all = pageRules(pageKey(ui.view, ui.pack));
  if (all === null || all.length === 0) { renderNotice(root); return; }
  if (page.root.parentNode !== root) root.replaceChildren(page.root);
  const pack = ui.view === "created" ? null : packById(ui.pack), shown = shownRules(all);
  renderHead(all, shown, pack);
  const controls = ruleControls();
  if (controls.root.parentNode !== page.controls) page.controls.replaceChildren(controls.root);
  if (page.packAll.parentNode !== controls.slot) controls.slot.replaceChildren(page.packAll);
  renderPackAll(all, pack?.name ?? CREATED);
  renderRuleTable(page.table, shown);
}

/** The whole-pack switch: a mixed or all-off page turns every rule on, an all-on page turns them all off. */
function togglePage() {
  const all = pageRules(pageKey(ui.view, ui.pack));
  if (all === null || all.length === 0) return;
  const turnOn = switchTurnsOn(packSwitch(all).state), pack = ui.view === "created" ? null : packById(ui.pack);
  const label = `${turnOn ? "On" : "Off"} ${ownerWords()}: ${all.length === 1 ? "its rule" : `all ${fmt(all.length)} rules`} in ${pack?.name ?? CREATED}.`;
  // A pack's rules follow one setting for the pack; rules created by you have no pack, so each gets its own.
  if (pack) writes.setPackEnabled(pack.id, turnOn, label);
  else writes.setEnabled(all.map(r => r.rule.id), turnOn, label);
}

export const actions = { ...ruleControlActions, ptoggle: togglePage };
export const onInput = ruleControlInput;
export const onChange = ruleControlChange;
