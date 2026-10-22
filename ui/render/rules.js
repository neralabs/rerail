/**
 * A pack page's rule table (`#pack/<id>`, `#created`): the column labels Rule · Violations logged · On/off, then one
 * row per rule, reconciled in place on every poll. A row opens the rule's sheet; its switch turns the rule on or off.
 */
import { h, setAttr, setText, setClass, syncChildren } from "../dom.js";
import { fmt, plural, whenLabel } from "../format.js";
import { INTERNAL_TESTING, isInternalTesting, isRunning } from "../derive.js";
import { isMeasured, isAudited, fires, allFires, activityAllOf, rangeLabel, titleOf, panelRuleId } from "../state.js";

export const CREATE_COMMAND = "/rerail-create";

/* ---------- rows ---------- */

function createRow(stored) {
  const id = stored.rule.id;
  const title = h("button.t", { "data-act": "detail", "data-i": id });
  const count = h("span.count.num", null, h("b"));
  const sw = h("button.sw", { role: "switch", "aria-checked": "false", "data-act": "toggle", "data-i": id });
  const stopped = h("span.badge", { hidden: true }, "Not running");
  const internal = h("span.badge", { hidden: true, title: "Not part of Rerail 0.1.0: conversation rules are in internal testing." }, INTERNAL_TESTING);
  const folder = h("span.mono", { title: "Added at" });
  const last = h("span");
  const row = h("div.row", { "data-act": "detail", "data-i": id },
    h("div.rmain", null, h("div.rtitle", null, title), h("div.rside", null, count, sw),
      h("div.rmeta", null, internal, stopped, h("span.mono", null, id), folder, last)));
  row._r = { title, count, sw, internal, stopped, folder, last };
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
  setAttr(r.title, "title", title);
  const count = countContent(id);
  const b = r.count.firstChild;
  setText(b, count.text);
  setClass(b, "zero", count.zero);
  setAttr(r.count, "title", count.tip);
  setAttr(r.sw, "aria-checked", String(stored.enabled));
  setAttr(r.sw, "aria-label", `Turn ${title} ${stored.enabled ? "off" : "on"}`);
  setAttr(r.internal, "hidden", !isInternalTesting(stored));
  // The folders the rule is added at, when it is not the whole repository.
  setText(r.folder, stored.folders.filter(f => f !== "").join(", "));
  setAttr(r.folder, "hidden", stored.folders.every(f => f === ""));
  const lastSeen = activityAllOf(id).lastSeenAt;
  const lastText = allFires(id) && lastSeen ? `last logged ${whenLabel(lastSeen)}` : "";
  setText(r.last, lastText);
  setAttr(r.last, "hidden", lastText === "");
}

/* ---------- the table ---------- */

let table = null;

function build() {
  const list = h("div.list");
  const noMatch = h("div.list", { hidden: true }, h("div.empty", null, h("h3", null, "No matches"), h("p", null, "Try a different search or filter.")));
  // The column labels: the long one where there is room, "Violations" where it is crowded.
  const head = h("div.colhead", { "aria-hidden": "true" },
    h("span", null, "Rule"), h("span", null, h("span.long", null, "Violations logged"), h("span.short", null, "Violations")), h("span", null, "On/off"));
  table = { root: h("div.rtable", null, head, list, noMatch), head, list, noMatch };
}

/** Render `rules` (already filtered and sorted) into `root` as the rule table; none shown reads "No matches". */
export function renderRuleTable(root, rules) {
  if (table === null) build();
  if (table.root.parentNode !== root) root.replaceChildren(table.root);
  const none = rules.length === 0;
  setAttr(table.head, "hidden", none);
  setAttr(table.list, "hidden", none);
  setAttr(table.noMatch, "hidden", !none);
  syncChildren(table.list, rules, r => r.rule.id, createRow, updateRow);
}
