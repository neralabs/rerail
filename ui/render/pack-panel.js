/**
 * One rule pack in the sheet: what it covers and its size, Add pack in the header until it is added, once added how
 * its rules treat linter suppression comments with a switch for the whole pack, its rules A to Z (with a filter box
 * for a large pack), and Remove pack in a footer that stays in view. Also the pack badge and the Add / Remove control
 * the catalog and the pack page share.
 */
import { h, setContent, icon, searchIcon } from "../dom.js";
import { fmt, plural } from "../format.js";
import { categoryLabel, inPack, packSuppressions } from "../derive.js";
import { data, ui, isAttached, keptPacks, reviewing, repoName } from "../state.js";
import { SHEET_FILTER_AT, sheetRules } from "../pack-model.js";
import { changeOwner } from "../shell.js";

/** Whether the pack is on where its button shows: kept, during the review of a repository not set up yet; else added. */
const packOn = pack => (reviewing() ? keptPacks().includes(pack.id) : isAttached(pack));

/** Remove pack's question, in place: the whole pack goes, its history stays. */
function removeQuestion(pack) {
  const n = pack.rules.length, repo = repoName() || "this repo";
  return h("span.rmask", { role: "alertdialog", "aria-label": `Remove ${pack.name}?` },
    h("span", null, `Remove ${n === 1 ? "the 1 rule" : `all ${fmt(n)} rules`} in ${pack.name} from ${repo}? ${n === 1 ? "It goes" : "They go"} off and the pack returns to Rule packs. History is kept.`),
    h("button.btn.sm.danger", { "data-act": "removepack", "data-i": pack.id }, "Remove"),
    h("button.btn.sm.ghost.link", { "data-act": "cancelremove" }, "Cancel"));
}

/**
 * Remove pack, which asks once in place before it acts; `quiet` draws it as a muted link that turns red on hover (a
 * list of packs, the pack page), else as the sheet's red button.
 */
export function removePackControl(pack, quiet = false) {
  if (ui.removeAsk === pack.id) return removeQuestion(pack);
  return h(`button.btn.sm.danger${quiet ? ".quiet" : ""}`, { "data-act": "removepack", "data-i": pack.id, title: "Its rules go off and the pack returns to Rule packs" }, "Remove pack");
}

/** Add pack while the pack is not on here; else Remove pack (`removePackControl`). */
export function packAction(pack, { solid = false, quiet = false } = {}) {
  if (!packOn(pack)) return h(`button.btn.sm${solid ? ".solid" : ""}`, { "data-act": "addpack", "data-i": pack.id }, icon("plus"), "Add pack");
  return removePackControl(pack, quiet);
}

/** The pack's category as people read it, e.g. "Language" or "Framework". */
export const packKind = pack => categoryLabel(pack.category);

/** The design's short marks for packs whose name or id does not shorten well. */
const PACK_LETTERS = { javascript: "JS", typescript: "TS", css: "CSS", complexity: "C6" };

/** A pack's badge: its short mark, else the first two letters of its id ("python" → "PY"). */
const monogram = pack => PACK_LETTERS[pack.id] ?? pack.id.replace(/[^a-z0-9]/giu, "").slice(0, 2).toUpperCase();

export const packIcon = pack => h("span.picon", { "aria-hidden": "true" }, h(`span.ltr${monogram(pack).length > 2 ? ".l3" : ""}`, null, monogram(pack)));

const MODE_NAME = { log: "Log", steer: "Steer", block: "Block" };
/** How a pack rule starts once added: its mode, and "off" for a rule the pack adds switched off. */
const startsAs = rule => `${MODE_NAME[rule.mode] ?? rule.mode}${rule.on ? "" : " · off"}`;

/** Once the pack is added: whether its rules here honour suppression comments, and the pack's switch for them. */
function suppressionsLine(pack, suppressions) {
  if (suppressions === null) return null;
  return h("p.pintro", null, `Linter suppression comments, such as eslint-disable or noqa: ${suppressions.text}. `,
    h("button.btn.sm.ghost.link", { "data-act": "packsuppress", "data-i": pack.id, "data-v": suppressions.next },
      suppressions.next === "ignore" ? "Ignore them" : "Honour them"));
}

/* ---------- the sheet ---------- */

let sheet = null;

function build() {
  const head = h("div.phead");
  const intro = h("div");
  const input = h("input", { id: "pkq", type: "search", "aria-label": "Filter rules in this pack" });
  const filter = h("div.search.pk", null, searchIcon(14), input);
  const list = h("div.pgrp.flat");
  const foot = h("div.pfoot");
  const root = h("div.pwrap.pack", null, head, h("div.pbody", null, intro, filter, list, foot));
  sheet = { root, head, intro, input, filter, list, foot, pack: null, packId: null, query: "" };
  // The box filters this sheet's list as the person types; the rest of the page does not change.
  input.oninput = () => { sheet.query = input.value; renderList(sheet.pack); };
}

function renderHead(pack, on) {
  const n = pack.rules.length;
  setContent(sheet.head, JSON.stringify(["head", pack.id, on]), () => [
    h("div.ptop", null, packIcon(pack), h("div", null, h("div.eyebrow", null, `Rule pack · ${packKind(pack)}`), h("h2", null, pack.name)),
      h("button.x", { "data-act": "close", "aria-label": "Close" }, "✕")),
    h("div.pact", null, h("span.muted", null, `${fmt(n)} ${plural(n, "rule")}${on ? " · added" : ""}`),
      h("span.grow"), on ? null : packAction(pack, { solid: true }))]);
}

function renderIntro(pack, on, suppressions) {
  setContent(sheet.intro, JSON.stringify(["intro", pack.id, on, suppressions]), () => [
    h("p.pintro", null, pack.covers, " ", on
      ? "Turn its rules on or off on its page in the menu."
      : "Adding it gives its rules a page in the menu. Each rule starts as listed below."),
    suppressionsLine(pack, suppressions)]);
}

function renderList(pack) {
  const on = packOn(pack), rows = sheetRules(pack, sheet.query);
  setContent(sheet.list, JSON.stringify(["list", pack.id, on, sheet.query, data.rules.size]), () => (rows.length === 0
    ? h("p.pintro.muted", null, `No rules match “${sheet.query.trim()}”.`)
    : rows.map(rule => h("div.prow", null,
      data.rules.has(rule.id) ? h("button.t", { "data-act": "packrule", "data-i": rule.id, title: rule.title }, rule.title) : h("span.t", { title: rule.title }, rule.title),
      on ? null : h("span.m", null, startsAs(rule)),
      h("span.mono", null, rule.id)))));
}

export function renderPackPanel(root, pack) {
  if (sheet === null) build();
  if (sheet.root.parentNode !== root) { root.replaceChildren(sheet.root); delete root.dataset.sig; }
  if (sheet.packId !== pack.id) { sheet.packId = pack.id; sheet.query = ""; sheet.input.value = ""; }
  sheet.pack = pack;
  const on = packOn(pack), added = isAttached(pack);
  // Every rule the pack added in some folder: a rule may also be added on its own elsewhere, and that folder comes first.
  const present = [...data.rules.values()].filter(r => inPack(r, pack.id));
  const suppressions = added && !reviewing() && present.length > 0 ? packSuppressions(present, pack.id, changeOwner(), data.checkout?.checkoutPath) : null;
  renderHead(pack, on);
  renderIntro(pack, on, suppressions);
  sheet.filter.hidden = pack.rules.length <= SHEET_FILTER_AT;
  sheet.input.placeholder = `Filter ${fmt(pack.rules.length)} rules`;
  renderList(pack);
  sheet.foot.hidden = !on;
  setContent(sheet.foot, JSON.stringify(["foot", pack.id, on, ui.removeAsk === pack.id, repoName()]), () => (on ? removePackControl(pack) : null));
}
