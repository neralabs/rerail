/**
 * Activity: every violation logged in the range, newest first, by day. Narrow to one rule pack with
 * the menu, or to one rule from a row (or from the rule panel's "See it in Activity").
 */
import { h, setAttr, setText, setContent, syncChildren, icon } from "../dom.js";
import { fmt, clock, dayLabel, daysAgo, shortSessionId } from "../format.js";
import { data, ui, rangeLabel, titleOf, attachedPackOf } from "../state.js";
import { isByRerail } from "../derive.js";
import { rangeControl, syncRange } from "./overview.js";

let pane = null;

const OTHER = "Other rules";
/** The rule pack a violation files under: the added pack its rule is present through, else Other or Created rules. */
const packNameOf = stored => (stored ? attachedPackOf(stored)?.name ?? (isByRerail(stored) ? OTHER : "Created rules") : OTHER);

function build() {
  const rows = h("div.list.recent-rows");
  const empty = h("div.list", { hidden: true }, h("div.empty", null, h("h3", null, "Nothing in this range"), h("p")));
  const more = h("div.more", { hidden: true }, h("button.btn.sm", { "data-act": "more" }));
  const lead = h("p");
  const packSelect = h("select", { id: "actpack", "aria-label": "Show one rule pack" });
  const range = rangeControl();
  pane = h("section.group.activity", null,
    h("div.ghead.top", null, h("h2", null, "Activity"), lead,
      h("div.act", null, h("label.sortctl", null, h("span", null, "Show"), packSelect), range)),
    empty, rows, more);
  pane._a = { rows, empty, more, lead, packSelect, range };
}

function createRow(item) {
  if (item.day !== undefined) return h("div.day", null, item.day);
  return h("div.ev", null,
    h("span.when.num"),
    h("div.what", null, h("span.oc"), h("span.msg"), h("small", null, h("button.lnk", { "data-act": "actrule" }), h("span.path"))),
    h("button.open", { "data-act": "session" }));
}

const pathLabel = x => (x.path ? `${x.path}${x.span ? `:${x.span.startLine + 1}` : ""}` : "");

function updateRow(node, item) {
  if (item.day !== undefined) return;
  const x = item.x, stored = data.rules.get(x.ruleId);
  setText(node.querySelector(".when"), clock(x.at));
  // The mode the violation was reported with; the rule may have changed since. Block is shown as Steered (not in this version).
  const outcome = x.mode === "log" ? "logged" : "steered";
  const oc = node.querySelector(".oc");
  if (oc.dataset.o !== outcome) {
    oc.dataset.o = outcome;
    oc.className = `oc oc-${outcome}`;
    oc.replaceChildren(icon(outcome, 16));
    setAttr(oc, "title", outcome === "steered" ? "Steered: Rerail told the coding agent" : "Logged: Rerail recorded it; the coding agent wasn’t told");
  }
  setText(node.querySelector(".msg"), x.message);
  const lnk = node.querySelector(".lnk");
  setText(lnk, stored ? titleOf(stored) : x.ruleId);
  setAttr(lnk, "data-i", x.ruleId);
  setAttr(lnk, "title", "Show only this rule");
  const path = pathLabel(x);
  setText(node.querySelector(".path"), path ? ` · ${path}` : "");
  const open = node.querySelector(".open");
  setAttr(open, "data-i", x.sessionId);
  setText(open, shortSessionId(x.sessionId, data.sessions.find(s => s.sessionId === x.sessionId)?.host));
}

function syncPackSelect(select, findings) {
  const counts = new Map();
  for (const x of findings) { const n = packNameOf(data.rules.get(x.ruleId)); counts.set(n, (counts.get(n) ?? 0) + 1); }
  const options = [["", `All rule packs · ${fmt(findings.length)}`], ...[...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).map(([n, c]) => [n, `${n} · ${fmt(c)}`])];
  const sig = JSON.stringify(options);
  if (select.dataset.sig !== sig) {
    select.dataset.sig = sig;
    select.replaceChildren(...options.map(([v, l]) => h("option", { value: v }, l)));
  }
  if (select.value !== ui.actPack) select.value = ui.actPack;
}

export function renderActivity(root) {
  if (pane === null) build();
  if (pane.parentNode !== root) root.replaceChildren(pane);
  const a = pane._a, label = rangeLabel();
  syncRange(a.range);
  syncPackSelect(a.packSelect, data.findings);
  const ruleName = ui.actRule === null ? null : (data.rules.get(ui.actRule) ? titleOf(data.rules.get(ui.actRule)) : ui.actRule);
  setContent(a.lead, JSON.stringify([label, ruleName]), () => [`Every violation logged ${label}, newest first.`,
    ruleName === null ? null : h("button.chip", { "data-act": "actclear", "aria-pressed": "true", title: "Show every rule again" }, `${ruleName} ✕`)]);
  const list = data.findings.filter(x => (!ui.actPack || packNameOf(data.rules.get(x.ruleId)) === ui.actPack) && (ui.actRule === null || x.ruleId === ui.actRule));
  const filtered = ui.actPack !== "" || ui.actRule !== null;
  const total = filtered ? list.length : Math.max(data.findingsTotal, list.length);
  setAttr(a.empty, "hidden", total > 0);
  setText(a.empty.querySelector("p"), `Rerail audited the sessions ${label} and logged no violations${filtered ? " for this filter" : ""}. Widen the range above to see earlier ones.`);
  setAttr(a.rows, "hidden", total === 0);
  const shown = list.slice(0, ui.recentShown);
  const items = [];
  let lastDay = null;
  for (const x of shown) {
    const day = dayLabel(daysAgo(x.at));
    if (day !== lastDay) { items.push({ key: `day:${day}`, day }); lastDay = day; }
    items.push({ key: `${x.auditId}:${x.id}`, x });
  }
  syncChildren(a.rows, items, i => i.key, createRow, updateRow);
  const remaining = total - shown.length;
  setAttr(a.more, "hidden", remaining <= 0);
  setText(a.more.firstChild, `Show ${fmt(Math.min(50, remaining))} more of ${fmt(total)}`);
}
