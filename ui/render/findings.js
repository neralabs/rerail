import { h, setAttr, setText, syncChildren } from "../dom.js";
import { fmt, clock, dayLabel, daysAgo, shortSessionId } from "../format.js";
import { ruleTitle } from "../derive.js";
import { data, ui, rangeLabel } from "../state.js";

let pane = null;

function build() {
  const rows = h("div.list.recent-rows");
  const empty = h("div.list", { hidden: true }, h("div.empty", null, h("h3", null, "Nothing in this range"), h("p")));
  const more = h("div.more", { hidden: true }, h("button.btn.sm", { "data-act": "more" }));
  pane = h("section.group", null,
    h("div.ghead", null, h("h2", null, "Recent findings"), h("p")),
    empty, rows, more);
  pane._rows = rows; pane._empty = empty; pane._more = more; pane._lead = pane.querySelector(".ghead p");
}

function createRow(item) {
  if (item.day !== undefined) return h("div.day", null, item.day);
  const x = item.x;
  return h("div.ev", null,
    h("span.when.num", null, clock(x.at)),
    h("div.what", null, x.message, h("small", null, h("span.rule"), h("span.path.mono", { hidden: true }), h("span.fix.mono", { hidden: true }))),
    h("button.open", { "data-act": "session", "data-i": x.sessionId }));
}

function pathLabel(x) {
  if (!x.path) return "";
  const line = x.span?.startLine ? `:${x.span.startLine}` : "";
  return `${x.path}${line}`;
}

function updateRow(node, item) {
  if (item.day !== undefined) return;
  const x = item.x, stored = data.rules.get(x.ruleId);
  const path = pathLabel(x);
  setText(node.querySelector(".rule"), stored ? ruleTitle(stored) : x.ruleId);
  const p = node.querySelector(".path");
  setText(p, path ? ` · ${path}` : "");
  setAttr(p, "hidden", path === "");
  const fix = node.querySelector(".fix");
  const fixMore = x.fix?.text.includes("\n") ? " …" : "";
  setText(fix, x.fix ? ` · suggested fix: ${x.fix.text.split("\n")[0]}${fixMore}` : "");
  setAttr(fix, "hidden", !x.fix);
  const host = data.sessions.find(s => s.sessionId === x.sessionId)?.host;
  setText(node.querySelector(".open"), shortSessionId(x.sessionId, host));
}

export function renderFindings(root) {
  if (pane === null) build();
  if (pane.parentNode !== root) root.append(pane);
  setAttr(pane, "hidden", ui.tab !== "recent");
  if (ui.tab !== "recent") return;
  const label = rangeLabel();
  setText(pane._lead, `Every time a rule triggered ${label}, newest first. Change the range in the overview above.`);
  const list = data.findings;
  const total = Math.max(data.findingsTotal, list.length);
  setAttr(pane._empty, "hidden", total > 0);
  setText(pane._empty.querySelector("p"), `Nera checked the sessions ${label} and no rules triggered. Widen the range above to see earlier findings.`);
  setAttr(pane._rows, "hidden", total === 0);
  const shown = list.slice(0, ui.recentShown);
  const items = [];
  let lastDay = null;
  for (const x of shown) {
    const day = dayLabel(daysAgo(x.at));
    if (day !== lastDay) { items.push({ key: `day:${day}`, day }); lastDay = day; }
    items.push({ key: `${x.runId}:${x.id}`, x });
  }
  syncChildren(pane._rows, items, i => i.key, createRow, updateRow);
  const remaining = total - shown.length;
  setAttr(pane._more, "hidden", remaining <= 0);
  setText(pane._more.firstChild, `Show ${fmt(Math.min(50, remaining))} more of ${fmt(total)}`);
}

/** Detach the pane from `root`; state is kept so the next render re-attaches it. */
export function unmountFindings(root) {
  if (pane !== null && pane.parentNode === root) pane.remove();
}
