import { h, setAttr, setText, setClass, setStyle, chevron, syncChildren, clearKeyed } from "../dom.js";
import { fmt, plural, whenLabel, clock, dayLabel, daysAgo, shortSessionId } from "../format.js";
import { originOf, ruleTitle, isNew } from "../derive.js";
import { data, ui, rangeLabel, fires, allFires, rerails, isMeasured, activityOf, activityAllOf, ruleFindings, originGroups, prefixGroups } from "../state.js";

const SORTS = [["triggered", "Most triggered"], ["recent", "Most recently triggered"], ["newest", "Newest first"], ["az", "A to Z"]];

function sortControl() {
  return h("label.sortctl", null, h("span", null, "Sort"),
    h("select", { "data-sort": "rules", "aria-label": "Sort" }, SORTS.map(([v, l]) => h("option", { value: v }, l))));
}

const listHead = () => h("div.lhead", null, h("span"), h("span", null, "Rule"), h("span.hs", null, h("span", null, "Activity"), h("span", null, "On")));

/** One rule row, built once; `updateRow` patches it on every render. */
function createRow(stored, compact) {
  const id = stored.rule.id;
  const row = h("div.row", { "data-id": id, "data-open": "false" });
  const caret = compact ? h("span") : h("button.caret", { "data-act": "rtoggle", "data-i": id, "aria-expanded": "false", "aria-label": "Show findings" }, chevron());
  const title = h("button.t", { "data-act": "detail", "data-i": id });
  const badgeNew = h("span.badge.new", { hidden: true }, "new");
  const count = h("span.count.num");
  const bar = h("span.bar", null, h("i"));
  const sw = h("button.sw", { role: "switch", "aria-checked": "false", "data-act": "toggle", "data-i": id });
  const origin = h("span.badge");
  const last = h("span");
  const main = h("div.rmain.exp", null, caret, h("div.rtitle", null, title, badgeNew),
    h("div.rside", null, count, bar, sw),
    h("div.rmeta", null, origin, h("span.mono", null, id), last));
  row.append(main);
  const children = compact ? null : h("div.children");
  if (children) row.append(children);
  row._r = { caret, title, badgeNew, count, bar: bar.firstChild, barWrap: bar, sw, origin, last, children };
  return row;
}

function countContent(id, f, all) {
  if (!isMeasured(id)) return [h("span.dim", null, "not measured")];
  if (f) return [h("b", null, fmt(f)), ` ${plural(f, "trigger")}`];
  if (all) return [h("span.dim", null, h("b.dim", null, "0"), ui.range === 0 ? " today" : " in range")];
  return [h("span.dim", null, "never triggered")];
}

function updateRow(row, stored) {
  const id = stored.rule.id, r = row._r, f = fires(id), all = allFires(id), rr = rerails(id), runs = activityOf(id).audits;
  const title = ruleTitle(stored);
  setClass(row, "off", !stored.enabled);
  setAttr(row, "aria-selected", ui.selectedRule === id ? "true" : null);
  const open = r.children !== null && ui.openRows.has(id);
  setAttr(row, "data-open", String(open));
  if (r.caret.tagName === "BUTTON") setAttr(r.caret, "aria-expanded", String(open));
  setText(r.title, title);
  setAttr(r.badgeNew, "hidden", !isNew(stored));
  const sig = `${isMeasured(id)}|${f}|${all}|${ui.range}`;
  if (r.count.dataset.sig !== sig) { r.count.dataset.sig = sig; r.count.replaceChildren(...countContent(id, f, all)); }
  setAttr(r.barWrap, "hidden", !isMeasured(id));
  // Share of runs in range that evaluated the rule and reported a violation, not a rank against the busiest rule.
  const share = runs ? Math.min(100, Math.round((100 * f) / runs)) : 0;
  setAttr(r.barWrap, "title", runs ? `${f} ${plural(f, "violation")} in ${runs} ${plural(runs, "run")} (${share}%)` : "not run in range");
  setStyle(r.bar, "width", `${share}%`);
  setAttr(r.sw, "aria-checked", String(stored.enabled));
  setAttr(r.sw, "aria-label", `Toggle ${title}`);
  const ob = originOf(stored);
  setAttr(r.origin, "class", `badge ${ob.cls}`);
  setText(r.origin, ob.label);
  const lastSeen = activityAllOf(id).lastSeenAt;
  const lastText = lastSeenText(rr, all, lastSeen);
  setText(r.last, lastText);
  setAttr(r.last, "hidden", lastText === "");
  if (r.children !== null) updateChildren(r.children, stored, f, all, lastSeen, open);
}

function lastSeenText(rr, all, lastSeen) {
  if (rr) {
    const tail = lastSeen ? ` · last re-rail ${whenLabel(lastSeen)}` : "";
    return `${fmt(rr)} ${plural(rr, "re-rail")}${tail}`;
  }
  if (all && lastSeen) return `last triggered ${whenLabel(lastSeen)}`;
  return "";
}

function pathLabel(x) {
  if (!x.path) return "";
  const line = x.span?.startLine ? `:${x.span.startLine}` : "";
  return `${x.path}${line}`;
}

function findingRow(x) {
  return h("div.ev", { "data-key": `${x.auditId}:${x.id}` },
    h("span.when", null, dayLabel(daysAgo(x.at)) === "Today" ? clock(x.at) : whenLabel(x.at)),
    h("div.what", null, x.message, h("small.mono", { hidden: !x.path }, pathLabel(x))),
    h("button.open", { "data-act": "session", "data-i": x.sessionId }, shortSessionId(x.sessionId, sessionHost(x.sessionId))));
}
const sessionHost = id => data.sessions.find(s => s.sessionId === id)?.host;

function emptyChildText(stored, all, lastSeen) {
  if (!stored.enabled) return "Off. Not checking.";
  if (!isMeasured(stored.rule.id)) return "Conversation rule. Triggers are not measured yet.";
  if (!all) return "On and in scope. No triggers so far.";
  const last = lastSeen ? `; last ${whenLabel(lastSeen)}` : "";
  return `No triggers ${rangeLabel()}. ${fmt(all)} before that${last}.`;
}

function childItems(stored, f, all, lastSeen) {
  if (!f) return [{ key: "empty", text: emptyChildText(stored, all, lastSeen) }];
  const list = ruleFindings(stored.rule.id).slice(0, 3);
  const items = list.map(x => ({ key: `${x.auditId}:${x.id}`, x }));
  if (list.length < f) items.push({ key: "more", more: f - list.length });
  return items;
}

function createChild(i, id) {
  if (i.x) return findingRow(i.x);
  if (i.more !== undefined) {
    return h("div.ev", null, h("span.when"), h("div.what.dim", null, "and ", h("span.n-more"), " earlier · ", h("button.open", { "data-act": "detail", "data-i": id }, "open the rule")), h("span"));
  }
  return h("div.ev", null, h("span.when"), h("div.what.dim"), h("span"));
}

function updateChild(node, i) {
  if (i.more !== undefined) setText(node.querySelector(".n-more"), fmt(i.more));
  else if (i.text !== undefined) setText(node.querySelector(".what"), i.text);
}

function updateChildren(el, stored, f, all, lastSeen, open) {
  if (!open) {
    if (el.childElementCount) clearKeyed(el);
    return;
  }
  const id = stored.rule.id;
  syncChildren(el, childItems(stored, f, all, lastSeen), i => i.key, i => createChild(i, id), updateChild);
}

function syncRows(container, rules, compact) {
  syncChildren(container, rules, r => r.rule.id, r => createRow(r, compact), updateRow);
}

/* ---------- panes ---------- */

let tabs = null, originPane = null, prefixPane = null;

function buildTabs() {
  tabs = h("div.rtabs", { role: "tablist" },
    h("button", { role: "tab", "aria-selected": "true", "data-act": "rtab", "data-i": "rules" }, "Rules ", h("span.n")),
    h("button", { role: "tab", "aria-selected": "false", "data-act": "rtab", "data-i": "recent" }, "Recent findings ", h("span.n")),
    h("span.groupby", null, h("span", null, "Group by"),
      h("span.seg2", { role: "group", "aria-label": "Group rules by" },
        h("button", { "data-act": "groupby", "data-i": "origin", "aria-pressed": "true" }, "Origin"),
        h("button", { "data-act": "groupby", "data-i": "prefix", "aria-pressed": "false" }, "Prefix"))));
  return tabs;
}

function groupSection(title, lead, act) {
  const rows = h("div.rows");
  const empty = h("div.empty", { hidden: true }, h("h3", null, "No matches"), h("p", null, "Try a different search or filter."));
  const section = h("section.group", null,
    h("div.ghead", null, h("h2", null, title), h("p", null, lead), h("div.act", null, act)),
    h("div.list", null, listHead(), rows, empty));
  section._rows = rows; section._empty = empty;
  return section;
}

function buildOriginPane() {
  const mine = groupSection("Your rules", "Compiled from your files or added through your agent. On from the start.", sortControl());
  const presets = groupSection("Nera presets", "Bundled with Nera. Off until you turn them on.", h("button.btn.sm", { "data-act": "presets-on" }, "Turn on all shown"));
  originPane = h("div", null, mine, presets);
  originPane._mine = mine; originPane._presets = presets;
  return originPane;
}

function buildPrefixPane() {
  const groups = h("div");
  const empty = h("div.list", { hidden: true }, h("div.empty", null, h("h3", null, "No matches"), h("p", null, "Try a different search or filter.")));
  prefixPane = h("div", null,
    h("div.ghead", null, h("h2", null, "By prefix"), h("p"), h("div.act", null, sortControl())),
    groups, empty);
  prefixPane._groups = groups; prefixPane._empty = empty; prefixPane._lead = prefixPane.querySelector(".ghead p");
  return prefixPane;
}

function createGroup(g) {
  const rows = h("div.rows");
  const section = h("section.group.topic", null, h("div.list", null,
    h("div.thead", null, h("span"), h("h3", null, h("span.name"), " ", h("span.n")),
      h("div.tside", null, h("span.tc", null, h("b.on"), " of ", h("span.total"), " on", h("span.act")),
        h("button.sw", { role: "checkbox", "data-act": "gtoggle", "data-i": g.key }))),
    rows));
  section._rows = rows;
  return section;
}

function groupState(g) {
  if (g.onN === 0) return "false";
  if (g.onN === g.all.length) return "true";
  return "mixed";
}

function updateGroup(section, g) {
  const head = section.querySelector(".thead");
  setText(head.querySelector(".name"), g.key);
  setText(head.querySelector(".n"), fmt(g.all.length));
  setText(head.querySelector("b.on"), fmt(g.onN));
  setText(head.querySelector(".total"), fmt(g.all.length));
  const act = head.querySelector(".act");
  if (g.act) { act.replaceChildren(" · ", h("b", null, fmt(g.act)), ` ${plural(g.act, "trigger")}`); } else act.replaceChildren();
  const sw = head.querySelector(".sw");
  const state = groupState(g);
  setAttr(sw, "aria-checked", state);
  const partial = state === "mixed" ? `, ${fmt(g.onN)} of ${fmt(g.all.length)} on` : "";
  setAttr(sw, "aria-label", `Turn ${g.key} rules ${state === "true" ? "off" : "on"}${partial}`);
  syncRows(section._rows, g.shown, true);
}

function syncTabs(root, sum) {
  if (root.firstChild !== tabs) root.prepend(tabs);
  const [tRules, tRecent] = tabs.querySelectorAll("[role=tab]");
  setAttr(tRules, "aria-selected", String(ui.tab === "rules"));
  setAttr(tRecent, "aria-selected", String(ui.tab === "recent"));
  setText(tRules.querySelector(".n"), fmt(sum.rules.length));
  setText(tRecent.querySelector(".n"), fmt(data.findingsTotal));
  setAttr(tabs.querySelector(".groupby"), "hidden", ui.tab !== "rules");
  for (const b of tabs.querySelectorAll("[data-act=groupby]")) setAttr(b, "aria-pressed", String(b.dataset.i === ui.groupBy));
  for (const s of root.querySelectorAll("select[data-sort]")) if (s.value !== ui.sort) s.value = ui.sort;
}

function renderOriginPane() {
  const { mine, presets } = originGroups();
  syncRows(originPane._mine._rows, mine, false);
  const filtered = ui.filter !== "all" || ui.query !== "";
  const empty = originPane._mine._empty;
  setAttr(empty, "hidden", mine.length > 0);
  setText(empty.querySelector("h3"), filtered ? "No matches" : "Nothing of yours yet");
  setText(empty.querySelector("p"), filtered ? "Try a different search or filter." : "Rules compiled from your instruction files or added through your agent show here.");
  setAttr(originPane._mine.querySelector(".lhead"), "hidden", mine.length === 0);
  syncRows(originPane._presets._rows, presets, false);
  setAttr(originPane._presets, "hidden", presets.length === 0);
}

function renderPrefixPane(sum) {
  const groups = prefixGroups();
  setText(prefixPane._lead, `${groups.length} ${plural(groups.length, "prefix", "prefixes")} · ${fmt(sum.on.length)} of ${fmt(sum.rules.length)} rules on.`);
  syncChildren(prefixPane._groups, groups, g => g.key, createGroup, updateGroup);
  setAttr(prefixPane._empty, "hidden", groups.length > 0);
}

/** Render the rules tab into `root`; `sum` is the per-render summary. */
export function renderRules(root, sum) {
  if (tabs === null) buildTabs();
  if (originPane === null) buildOriginPane();
  if (prefixPane === null) buildPrefixPane();
  syncTabs(root, sum);
  const byOrigin = ui.groupBy === "origin";
  const pane = byOrigin ? originPane : prefixPane;
  const other = byOrigin ? prefixPane : originPane;
  if (other.parentNode === root) other.remove();
  if (pane.parentNode !== root) tabs.after(pane);
  setAttr(pane, "hidden", ui.tab !== "rules");
  if (ui.tab !== "rules") return;
  if (byOrigin) renderOriginPane();
  else renderPrefixPane(sum);
}

/** Detach the tab strip and panes from `root`; state is kept so the next render re-attaches them. */
export function unmountRules(root) {
  for (const el of [tabs, originPane, prefixPane]) if (el !== null && el.parentNode === root) el.remove();
}
