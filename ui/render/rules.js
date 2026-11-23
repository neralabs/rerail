/**
 * A pack page's rule table (`#pack/<id>`, `#created`): the column labels Rule · Violations · Enforcement · On/off, then one row per
 * rule, reconciled in place on every poll. A row opens the rule's sheet; its switch turns the rule on or off.
 */
import { h, setAttr, setText, setClass, syncChildren } from "../dom.js";
import { fmt, plural, whenLabel } from "../format.js";
import { INTERNAL_TESTING, isInternalTesting, isLog, isRunning, notRunningReason, violationOutcomes } from "../derive.js";
import { data, isMeasured, isAudited, fires, allFires, activityOf, activityAllOf, rangeLabel, titleOf, panelRuleId } from "../state.js";
import { enforcementTag, ruleTableHead, updateEnforcementTag } from "./enforcement.js";

export const CREATE_COMMAND = "/rerail-create";

/* ---------- rows ---------- */

function createRow(stored) {
  const id = stored.rule.id;
  const title = h("button.t", { "data-act": "detail", "data-i": id });
  const count = h("span.count.num", null, h("b"));
  const sw = h("button.sw", { role: "switch", "aria-checked": "false", "data-act": "toggle", "data-i": id });
  const enforcement = enforcementTag();
  const stopped = h("span.badge", { hidden: true }, "Not running");
  const internal = h("span.badge", { hidden: true, title: "Not in this release." }, INTERNAL_TESTING);
  const folder = h("span.mono", { title: "Added at" });
  const last = h("span");
  const row = h("div.row", { "data-act": "detail", "data-i": id },
    h("div.rmain", null, h("div.rtitle", null, title), h("div.rside", null, count, enforcement, sw),
      h("div.rmeta", null, internal, stopped, h("span.mono", null, id), folder, last)));
  row._r = { title, count, sw, enforcement, internal, stopped, folder, last };
  return row;
}

/**
 * A count's hover: `f` violations in the range, and that the agent wasn't told when every one is Log only (one whose
 * steer was declined before the rule went to Log was told).
 */
export function loggedTip(stored, f) {
  return isLog(stored) && !stored.mixed && violationOutcomes(activityOf(stored.rule.id)).logged === f
    ? `${fmt(f)} logged ${rangeLabel()}. Agent not told.`
    : `${fmt(f)} ${plural(f, "violation")} logged ${rangeLabel()}`;
}

/** The violation count: a number, 0 when audited and clean, a dash when it has never been audited. */
function countContent(stored) {
  const id = stored.rule.id;
  if (!isMeasured(id)) return { text: "–", zero: true, tip: "Not counted in this release." };
  const f = fires(id);
  if (f) return { text: fmt(f), zero: false, tip: loggedTip(stored, f) };
  if (isAudited(id)) return { text: "0", zero: true, tip: `None logged ${rangeLabel()}` };
  return { text: "–", zero: true, tip: "Not run yet." };
}

/** The "Not running" badge: a rule switched on that Rerail runs nowhere, and why in its hover. */
function updateStopped(badge, stored) {
  // Switched on, but its stored options no longer apply to its document, or Rerail never evaluates it: it runs nowhere.
  // A conversation rule is in internal testing: it runs only while the daemon runs that experiment.
  const experimentOff = isInternalTesting(stored) && !(data.health?.experiments ?? []).includes("CONVERSATION_RULES");
  setAttr(badge, "hidden", (isRunning(stored) && !experimentOff) || !stored.enabled);
  setAttr(badge, "title", experimentOff
    ? "Conversation rules are experimental and off: start the Rerail daemon with RERAIL_EXPERIMENTAL_CONVERSATION_RULES=1 to run them."
    : notRunningReason(stored));
}

function updateRow(row, stored) {
  const id = stored.rule.id, r = row._r, title = titleOf(stored);
  setClass(row, "off", !isRunning(stored));
  updateStopped(r.stopped, stored);
  setAttr(row, "aria-selected", panelRuleId() === id ? "true" : null);
  setText(r.title, title);
  setAttr(r.title, "title", title);
  const count = countContent(stored);
  const b = r.count.firstChild;
  setText(b, count.text);
  setClass(b, "zero", count.zero);
  setAttr(r.count, "title", count.tip);
  updateEnforcementTag(r.enforcement, stored);
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
  const noMatch = h("div.list", { hidden: true }, h("div.empty", null, h("h3", null, "No matches."), h("p", null, "Try a different search or filter.")));
  const head = ruleTableHead();
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
