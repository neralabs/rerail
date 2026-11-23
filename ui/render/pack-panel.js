/**
 * One rule pack in the sheet: what it covers and its size, Add pack in the header until it is added, why Rerail
 * suggested it for this repository when it did, once added how its rules treat linter suppression comments, its rules
 * (with a filter box for a large pack), and Remove pack in a footer that stays in view. Until the pack is added its
 * rules are grouped by how they start, each with its Steer or Log only tag; once added they run A to Z. Also the pack
 * badge, the Add / Remove control and the "Why suggested" box the catalog and the pack page share.
 */
import { h, setContent, icon, searchIcon, pendingIcon } from "../dom.js";
import { fmt, plural } from "../format.js";
import { awaitingApproval, categoryLabel, inPack, isRerailPack, isWorktreePack, packSuppressions, publisherLabel } from "../derive.js";
import { data, ui, isAttached, keptPacks, reviewing, repoName } from "../state.js";
import { SHEET_FILTER_AT, sheetRules, startGroups, suggestedReason } from "../pack-model.js";

/** Whether the pack is on where its button shows: kept, during the review of a repository not set up yet; else added. */
const packOn = pack => (reviewing() ? keptPacks().includes(pack.id) : isAttached(pack));

/** Remove pack's question, in place: the whole pack goes, its history stays. */
function removeQuestion(pack) {
  const repo = repoName() || "this repo";
  return h("span.rmask", { role: "alertdialog", "aria-label": `Remove ${pack.name}?` },
    h("span", null, `Remove ${pack.name} from ${repo}? History is kept.`),
    h("button.btn.sm.danger", { "data-act": "removepack", "data-i": pack.id }, "Remove"),
    h("button.btn.sm.ghost.link", { "data-act": "cancelremove" }, "Cancel"));
}

/**
 * Remove pack, which asks once in place before it acts; `quiet` draws it as a muted link that turns red on hover (the
 * catalog), else as the sheet's red button. The pack page draws its own outline button and shows only the question.
 * The worktree's own rules are no attachment to remove: the files in its `.rerail/rules` decide them.
 */
export function removePackControl(pack, quiet = false) {
  if (isWorktreePack(pack)) return null;
  if (ui.removeAsk === pack.id) return removeQuestion(pack);
  return h(`button.btn.sm.danger${quiet ? ".quiet" : ""}`, { "data-act": "removepack", "data-i": pack.id }, "Remove pack");
}

/** Add pack while the pack is not on here; else Remove pack (`removePackControl`). */
export function packAction(pack, { solid = false, quiet = false } = {}) {
  if (!packOn(pack) && !isWorktreePack(pack)) return h(`button.btn.sm${solid ? ".solid" : ""}`, { "data-act": "addpack", "data-i": pack.id }, icon("plus"), "Add pack");
  return removePackControl(pack, quiet);
}

/**
 * The pack's category as people read it, e.g. "Language" or "Framework", and for a third party's pack who publishes
 * it: "Language · By @acme".
 */
export const packKind = pack => (isRerailPack(pack) ? categoryLabel(pack.category) : `${categoryLabel(pack.category)} · ${publisherLabel(pack)}`);

/** The design's short marks for packs whose name or id does not shorten well. */
const PACK_LETTERS = { javascript: "JS", typescript: "TS", css: "CSS", complexity: "C6" };

/** A pack's badge: its short mark, else the first two letters of its id ("python" → "PY"). */
const monogram = pack => PACK_LETTERS[pack.id] ?? pack.id.replace(/[^a-z0-9]/giu, "").slice(0, 2).toUpperCase();

export const packIcon = pack => h("span.picon", { "aria-hidden": "true" }, h(`span.ltr${monogram(pack).length > 2 ? ".l3" : ""}`, null, monogram(pack)));

/**
 * Why Rerail suggested a pack for this repository, called out in the brand's light fill: `label` in small capitals with
 * the question mark, then the reason.
 */
export const suggestedBox = (label, reason) => h("div.whybox", null,
  h("span.whyhead", null, icon("question", 14), h("b", null, label)), h("span.whytext", null, reason));

/** Once the pack is added: whether its rules here honour suppression comments. */
function suppressionsLine(text) {
  if (text === null) return null;
  return h("p.pintro", null, `Linter suppression comments, such as eslint-disable or noqa: ${text}.`);
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

/** While the pack waits for a person's approval: what that means, and Approve for what this version ships. */
function approvalLine(pack) {
  if (!awaitingApproval(pack)) return null;
  const { kind, id, digest } = pack.approval.subject;
  return h("div.welcome.warn", { role: "status" }, pendingIcon(), h("div", null,
    h("b", null, "Waits for approval."),
    h("p", null, "Rerail did not publish this version, so its rules run in Log, steering no one, until a person approves what it ships. Read what its rules tell the coding agent first."),
    h("p.acts", null, h("button.btn.sm.solid", { "data-act": "approve", "data-i": id, "data-kind": kind, "data-digest": digest }, "Approve"))));
}

/** Why Rerail suggested the pack for this repository, when it did and said why. */
function sheetWhy(reason) {
  return reason ? suggestedBox(`Suggested for ${repoName() || "this repo"}`, reason) : null;
}

function renderIntro(pack, on, suppressions) {
  const reason = suggestedReason(data.recommendations, pack.id);
  setContent(sheet.intro, JSON.stringify(["intro", pack.id, on, suppressions, pack.approval ?? null, reason, repoName()]), () => [
    approvalLine(pack),
    sheetWhy(reason),
    h("p.pintro", null, pack.covers, " ", on
      ? "Turn its rules on or off on its page in the menu."
      : "Adding it gives its rules a page in the menu. Each rule starts as listed below."),
    suppressionsLine(suppressions)]);
}

/** The tag of how a pack rule starts once added, by its mode: Steer or Log only, as a pack page's Enforcement column. */
const STEER = { kind: "steer", glyph: "steered", label: "Steer", tip: "Steer: Rerail tells your coding agent as it works." };
const START_TAGS = {
  steer: STEER,
  block: { ...STEER, tip: "Set to Block, which steers for now." },
  log: { kind: "log", glyph: "logged", label: "Log only", tip: "Log only: Rerail records its violations; your coding agent isn’t told." },
};

function startTag(rule) {
  const tag = START_TAGS[rule.mode] ?? STEER;
  return h(`span.enftag.${tag.kind}`, { title: tag.tip }, icon(tag.glyph, 12), tag.label);
}

/** A rule Rerail keeps but never evaluates (`inactive`) says so, its hover why. */
const notRunning = rule => (rule.inactive ? h("span.m", { title: rule.inactive.reason ?? null }, "Not running") : null);

/**
 * One rule of the sheet: its title, which opens the rule once it is here, with its id under it; `tagged`, for a pack not
 * added, how it starts.
 */
function sheetRow(rule, tagged) {
  const title = data.rules.has(rule.id)
    ? h("button.t", { "data-act": "packrule", "data-i": rule.id, title: rule.title }, rule.title)
    : h("span.t", { title: rule.title }, rule.title);
  const text = h("span.ptext", null, title, h("span.mono", { title: rule.id }, rule.id));
  if (!tagged) return h("div.prow.prule", null, text);
  return h(`div.prow.prule${rule.on ? "" : ".startsoff"}`, null, text, notRunning(rule), startTag(rule));
}

/** A group's heading in the sheet of a pack not added: "Start on", its count, and what that means. */
const startHead = (text, n, line) => h("div.pstart", null, h("b", null, text, h("span.n2", null, fmt(n))), h("span", null, line));

/** The rows of a pack not added: the rules it starts on, then the ones it starts off, each group under its heading. */
function startList(rows) {
  const { on, off } = startGroups(rows);
  return [
    on.length ? [startHead("Start on", on.length, "On as soon as the pack is added."), on.map(rule => sheetRow(rule, true))] : null,
    off.length ? [startHead("Start off", off.length, "Turn any on from the pack’s page once it’s added."), off.map(rule => sheetRow(rule, true))] : null,
  ];
}

function listContent(rows, on) {
  if (rows.length === 0) return h("p.pintro.muted", null, `No rules match “${sheet.query.trim()}”.`);
  return on ? rows.map(rule => sheetRow(rule, false)) : startList(rows);
}

function renderList(pack) {
  const on = packOn(pack), rows = sheetRules(pack, sheet.query);
  setContent(sheet.list, JSON.stringify(["list", pack.id, on, sheet.query, data.rules.size]), () => listContent(rows, on));
}

export function renderPackPanel(root, pack) {
  if (sheet === null) build();
  if (sheet.root.parentNode !== root) { root.replaceChildren(sheet.root); delete root.dataset.sig; }
  if (sheet.packId !== pack.id) { sheet.packId = pack.id; sheet.query = ""; sheet.input.value = ""; }
  sheet.pack = pack;
  const on = packOn(pack), added = isAttached(pack);
  // Every rule the pack added in some folder: a rule may also be added on its own elsewhere, and that folder comes first.
  const present = [...data.rules.values()].filter(r => inPack(r, pack.id));
  const suppressions = added && !reviewing() && present.length > 0 ? packSuppressions(present) : null;
  renderHead(pack, on);
  renderIntro(pack, on, suppressions);
  sheet.filter.hidden = pack.rules.length <= SHEET_FILTER_AT;
  sheet.input.placeholder = `Filter ${fmt(pack.rules.length)} rules`;
  renderList(pack);
  sheet.foot.hidden = !on;
  setContent(sheet.foot, JSON.stringify(["foot", pack.id, on, ui.removeAsk === pack.id, repoName()]), () => (on ? removePackControl(pack) : null));
}
