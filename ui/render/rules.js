/**
 * W1: Created rules first, then one section per rule pack added here with its groups, then any rules by
 * Rerail added on their own. Sections, group bands and rows are reconciled in place on every poll.
 */
import { h, setAttr, setText, setClass, chevron, icon, syncChildren } from "../dom.js";
import { fmt, plural, whenLabel } from "../format.js";
import { isByRerail, isRunning } from "../derive.js";
import { isMeasured, isAudited, fires, allFires, activityAllOf, data, ui, rangeLabel, ruleSections, filtering, titleOf, originChip, panelRuleId } from "../state.js";

export const CREATE_COMMAND = "/rerail-create";

/* ---------- rows ---------- */

function createRow(stored) {
  const id = stored.rule.id;
  const title = h("button.t", { "data-act": "detail", "data-i": id });
  const count = h("span.count.num");
  const sw = h("button.sw", { role: "switch", "aria-checked": "false", "data-act": "toggle", "data-i": id });
  const origin = h("span.badge");
  const stopped = h("span.badge", { hidden: true }, "Not running");
  const folder = h("span.mono", { title: "Added at" });
  const last = h("span");
  const row = h("div.row", { "data-id": id },
    h("div.rmain", null, h("div.rtitle", null, title), h("div.rside", null, count, sw), h("div.rmeta", null, origin, stopped, h("span.mono", null, id), folder, last)));
  row._r = { title, count, sw, origin, stopped, folder, last };
  return row;
}

/** The violation count: a number, 0 when audited and clean, a dash when it has never been audited. */
function countContent(id) {
  if (!isMeasured(id)) return { text: "–", zero: true, tip: "Conversation rules are not counted here yet." };
  const f = fires(id);
  if (f) return { text: fmt(f), zero: false, tip: `${fmt(f)} ${plural(f, "violation")} logged ${rangeLabel()}` };
  if (isAudited(id)) return { text: "0", zero: true, tip: `None logged ${rangeLabel()}` };
  return { text: "–", zero: true, tip: "Not audited yet: no audit has run this rule" };
}

function updateRow(row, stored) {
  const id = stored.rule.id, r = row._r, title = titleOf(stored);
  setClass(row, "off", !isRunning(stored));
  // Switched on, but its stored options no longer apply to its document: Rerail runs it nowhere.
  setAttr(r.stopped, "hidden", isRunning(stored) || !stored.enabled);
  setAttr(r.stopped, "title", stored.configError ? `Its options don't apply: ${stored.configError}` : null);
  setAttr(row, "aria-selected", panelRuleId() === id ? "true" : null);
  setText(r.title, title);
  const count = countContent(id);
  const b = r.count.firstChild ?? r.count.appendChild(h("b"));
  setText(b, count.text);
  setClass(b, "zero", count.zero);
  setAttr(r.count, "title", count.tip);
  setAttr(r.sw, "aria-checked", String(stored.enabled));
  setAttr(r.sw, "aria-label", `Turn ${title} ${stored.enabled ? "off" : "on"}`);
  setAttr(r.origin, "class", `badge ${isByRerail(stored) ? "rerail" : "mine"}`);
  setText(r.origin, originChip(stored));
  // The folders the rule is added at, when it is not the whole repository.
  setText(r.folder, stored.folders.filter(f => f !== "").join(", "));
  setAttr(r.folder, "hidden", stored.folders.every(f => f === ""));
  const lastSeen = activityAllOf(id).lastSeenAt;
  const lastText = allFires(id) && lastSeen ? `last logged ${whenLabel(lastSeen)}` : "";
  setText(r.last, lastText);
  setAttr(r.last, "hidden", lastText === "");
}

/* ---------- group bands ---------- */

const groupKey = (section, group) => `${section.key}|${group.name}`;
const groupIsOpen = (section, group) => filtering() || (ui.groupOpen.get(groupKey(section, group)) ?? !ui.foldedSections.has(section.key));

function createBand(item) {
  const key = item.key.slice(5);
  return h("div.lsub", { "data-open": "true" },
    h("span.lbl", null, h("button.caret", { "data-act": "gfold", "data-i": key }, chevron()), h("span.name"), h("span.n")),
    h("span.lside", null, h("span.count.num", null, h("b"), h("span.u")),
      h("button.sw", { role: "switch", "data-act": "gtoggle", "data-i": key })));
}

function updateBand(node, { section, group }) {
  const open = groupIsOpen(section, group);
  // The count is of rules that run; the switch shows how they are set.
  const onN = group.all.filter(isRunning).length, switchedN = group.all.filter(r => r.enabled).length, total = group.all.length;
  setAttr(node, "data-open", String(open));
  const caret = node.querySelector(".caret");
  setAttr(caret, "aria-expanded", String(open));
  setAttr(caret, "aria-label", `${open ? "Collapse" : "Expand"} ${group.name}`);
  setText(node.querySelector(".name"), group.name);
  const badge = node.querySelector(".n");
  setText(badge, group.shown.length < total ? `${fmt(group.shown.length)} of ${fmt(total)} shown` : `${fmt(onN)} of ${fmt(total)} on`);
  setClass(badge, "mix", onN < total);
  const n = group.all.reduce((sum, r) => sum + fires(r.rule.id), 0);
  const counted = group.all.some(r => isMeasured(r.rule.id));
  setText(node.querySelector(".count b"), counted ? fmt(n) : "–");
  setClass(node.querySelector(".count b"), "zero", n === 0);
  setText(node.querySelector(".count .u"), counted ? `${plural(n, "violation")} logged` : "");
  setAttr(node.querySelector(".count"), "title", counted ? null : "Conversation rules are not counted here yet.");
  const state = switchedN === 0 ? "false" : switchedN === total ? "true" : "mixed";
  const sw = node.querySelector(".sw");
  setAttr(sw, "aria-checked", state);
  setAttr(sw, "aria-label", `Turn every rule in ${group.name} ${state === "true" ? "off" : "on"}`);
}

/* ---------- sections ---------- */

function sectionTitle(section) {
  if (section.kind === "mine") return "Created rules";
  if (section.kind === "legacy") return "Other rules by Rerail";
  return section.pack.name;
}

function sectionLead(section) {
  if (section.kind === "mine") return ["Written with your coding agent through ", h("span.mono", null, CREATE_COMMAND), "."];
  if (section.kind === "legacy") return ["Rules by Rerail added to this repo on their own, not through a rule pack it uses."];
  return [section.pack.covers];
}

function emptyMine() {
  return h("div.empty.mine", null,
    h("p", null, h("b", null, "Nothing created yet")),
    h("p", null, "Rule packs cover the common ground. The rules that matter most are the ones only your team would write. Run ",
      h("span.mono", null, CREATE_COMMAND), " in a session to write one."),
    h("p", null, h("button.btn.sm", { "data-act": "newrule" }, icon("plus"), "Create a rule")));
}

function createSection(section) {
  const name = h("span.name"), n2 = h("span.n2"), share = h("span.share");
  const lead = h("p");
  const foldGroups = h("button.btn.sm.ghost.link", { "data-act": "gfoldall", "data-i": section.key });
  const hide = h("button.btn.sm.ghost.link", { "data-act": "layer", "data-i": section.key });
  const list = h("div.list");
  const node = h("section.group.layer", null, h("div.ghead", null, h("h2", null, name, n2, share), lead, h("div.act", null, foldGroups, hide)), list);
  node._s = { name, n2, share, lead, foldGroups, hide, list };
  lead.append(...sectionLead(section));
  return node;
}

function listItems(section) {
  if (section.kind === "mine") {
    const [group] = section.groups;
    if (group.all.length === 0) return [{ key: "empty" }];
    return group.shown.map(r => ({ key: r.rule.id, r }));
  }
  const items = [];
  for (const group of section.groups) {
    if (group.shown.length === 0) continue;
    items.push({ key: `band:${groupKey(section, group)}`, section, group });
    if (groupIsOpen(section, group)) for (const r of group.shown) items.push({ key: r.rule.id, r });
  }
  return items;
}

function updateSection(node, section, total) {
  const s = node._s, all = section.groups.flatMap(g => g.all), onN = all.filter(isRunning).length;
  const visible = !ui.hiddenSections.has(section.key) || filtering();
  setText(s.name, sectionTitle(section));
  setText(s.n2, all.length ? `${fmt(onN)} of ${fmt(all.length)} on` : "0");
  setClass(s.n2, "mix", onN < all.length);
  const n = all.reduce((sum, r) => sum + fires(r.rule.id), 0);
  setAttr(s.share, "hidden", total === 0 || all.length === 0);
  setText(s.share, `${fmt(n)} of ${fmt(total)} violations logged`);
  setAttr(s.share, "title", `Violations logged in this section, of all violations ${rangeLabel()}`);
  setAttr(s.foldGroups, "hidden", !visible || section.kind === "mine");
  setText(s.foldGroups, ui.foldedSections.has(section.key) ? "Expand groups" : "Collapse groups");
  setText(s.hide, visible ? "Hide" : "Show");
  setAttr(s.hide, "aria-expanded", String(visible));
  setAttr(s.list, "hidden", !visible);
  const items = visible ? listItems(section) : [];
  syncChildren(s.list, items, i => i.key,
    i => (i.key === "empty" ? emptyMine() : i.group ? createBand(i) : createRow(i.r)),
    (el, i) => { if (i.group) updateBand(el, i); else if (i.r) updateRow(el, i.r); });
}

/* ---------- the view ---------- */

let pane = null;

function build() {
  const sections = h("div.sections");
  const noMatch = h("div.list", { hidden: true }, h("div.empty", null, h("h3", null, "No matches"), h("p", null, "Try a different search or filter.")));
  const noPacks = h("section.group.layer", { hidden: true },
    h("div.ghead", null, h("h2", null, "Rule packs", h("span.n2", null, "0 added")), h("p", null, "Rule packs by Rerail are off until you add them.")),
    h("div.list", null, h("div.empty", null, h("h3", null, "No rule packs added yet"),
      h("p", null, h("button.btn.sm", { "data-act": "catalog" }, icon("plus"), "Add rule packs")))));
  pane = h("div.rules", null, sections, noMatch, noPacks);
  pane._p = { sections, noMatch, noPacks };
}

export function renderRules(root, sum) {
  if (pane === null) build();
  if (pane.parentNode !== root) root.replaceChildren(pane);
  const p = pane._p, active = filtering();
  const sections = ruleSections().filter(s => !active || s.groups.some(g => g.shown.length));
  syncChildren(p.sections, sections, s => s.key, createSection, (node, s) => updateSection(node, s, sum.triggers));
  setAttr(p.noMatch, "hidden", !active || sections.length > 0);
  setAttr(p.noPacks, "hidden", active || !data.loaded || sections.some(s => s.kind !== "mine"));
}

/** "Collapse all groups" folds every section's groups; "Expand all groups" opens them again. */
export function allGroupsFolded() {
  const keys = ruleSections().filter(s => s.kind !== "mine").map(s => s.key);
  return keys.length > 0 && keys.every(k => ui.foldedSections.has(k));
}

export function sectionKeys() {
  return ruleSections().filter(s => s.kind !== "mine").map(s => s.key);
}

/** Every rule in one group band, by its `section|group` key. */
export function groupRules(key) {
  const [sectionKey, name] = [key.slice(0, key.indexOf("|")), key.slice(key.indexOf("|") + 1)];
  return ruleSections().find(s => s.key === sectionKey)?.groups.find(g => g.name === name)?.all ?? [];
}
