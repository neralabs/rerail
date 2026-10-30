/**
 * History by rule (`#history`; `#activity` before v8): the rules with violations in the range, most first, and the
 * chosen rule's violations, newest first by day, a page at a time. `?rule=<id>` (a rule's "See all", a steer link)
 * chooses the rule; without one the top rule shows. The All · Steered · Logged chips (`?outcome=`, the Violations
 * steered tile opens Steered) narrow the list to the violations recorded on Steer or Block, or on Log.
 */
import { h, setAttr, setText, setContent, syncChildren, icon } from "../dom.js";
import { fmt, plural, dayLabel, daysAgo, shortSessionId } from "../format.js";
import { data, ui, rangeLabel } from "../state.js";
import { MODE_LABEL, ruleMode, messageText } from "../derive.js";
import { loadMoreFindings, refreshNow } from "../poll.js";
import { invalidate, loadMore } from "../shell.js";
import { rangeControl, syncRange } from "../render/range.js";
import { OUTCOME_CHIPS, inRange, rankedRules, filterRanked, historyRuleId, historyTitle, historyOrigin, historyEmpty } from "../history-model.js";

export const needs = ["rules", "packs", "activity", "findings"];

/** The icon and colour of each mode's chip, as the rule sheet shows them. */
const MODE_OUTCOME = { log: "logged", steer: "steered", block: "blocked" };
/** A violation's time, 24-hour ("17:35"), as the design reads it. */
const CLOCK = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" });
const SVG_NS = "http://www.w3.org/2000/svg";
/** Phosphor bold glyphs on a 256 grid: the list toggle's lines, the search glass and the picker's caret. */
const LIST_PATH = "M228,128a12,12,0,0,1-12,12H40a12,12,0,0,1,0-24H216A12,12,0,0,1,228,128ZM40,76H216a12,12,0,0,0,0-24H40a12,12,0,0,0,0,24ZM216,180H40a12,12,0,0,0,0,24H216a12,12,0,0,0,0-24Z";
const SEARCH_PATH = "M232.49,215.51,185,168a92.12,92.12,0,1,0-17,17l47.53,47.54a12,12,0,0,0,17-17ZM44,112a68,68,0,1,1,68,68A68.07,68.07,0,0,1,44,112Z";
const CARET_PATH = "M216.49,104.49l-80,80a12,12,0,0,1-17,0l-80-80a12,12,0,0,1,17-17L128,159l71.51-71.52a12,12,0,0,1,17,17Z";
function glyph(d, size) {
  const svg = document.createElementNS(SVG_NS, "svg");
  for (const [k, v] of [["class", "ico"], ["width", String(size)], ["height", String(size)], ["viewBox", "0 0 256 256"], ["fill", "currentColor"], ["aria-hidden", "true"]]) svg.setAttribute(k, v);
  const path = document.createElementNS(SVG_NS, "path");
  path.setAttribute("d", d);
  svg.append(path);
  return svg;
}
let pane = null;
/** Hide list: the list column folds away, the chosen rule's violations take the width and its heading becomes a picker. */
let listHidden = false;
/** The list's search box. */
let query = "";

function build() {
  const lead = h("p");
  const toggle = h("button.hltoggle", { "data-act": "hlisttoggle" }, glyph(LIST_PATH, 14), h("span"));
  const range = rangeControl();
  const search = h("input", { id: "hq", type: "search", placeholder: "Search rules or packs", "aria-label": "Search rules" });
  const rows = h("div.hlist");
  const noMatch = h("div.empty.hnomatch", { hidden: true }, h("p", null, "No rules match."));
  const title = h("h2.clamp2");
  const pick = h("select", { id: "hpick", "aria-label": "Choose a rule" });
  const pickLabel = h("span.m", { "aria-hidden": "true" });
  const pickWrap = h("h2.hpick", { hidden: true }, h("span.sel", null, pickLabel, pick, glyph(CARET_PATH, 12)));
  const meta = h("div.l3");
  const kvs = h("dl.hkv");
  const summaryLine = h("div.hsum", null, h("span.tsum"));
  const violations = h("div.hrows");
  const more = h("div.more", { hidden: true }, h("button.btn.sm", { "data-act": "hmore" }));
  const nothing = h("div.empty.hnone", { hidden: true }, h("h3", null, "Nothing in this range"), h("p"));
  const empty = h("div.list.hempty", { hidden: true }, h("div.empty", null, h("h3"), h("p")));
  const chips = h("div.chips.hchips", { role: "group", "aria-label": "Outcome" },
    OUTCOME_CHIPS.map(([v, label]) => h("button.chip", { "data-act": "houtcome", "data-i": v, "aria-pressed": "false" }, label)));
  // A chip that leaves the list empty: what the section says instead of a rule.
  const unmatched = h("div.empty", { hidden: true }, h("h3"), h("p"));
  const detail = h("div.hdetail", null, h("div.l1", null, title, pickWrap), meta, kvs);
  const scroll = h("div.hscroll", null, nothing, violations, more);
  const ltrow = h("div.hltrow", null, toggle);
  const aside = h("aside", null,
    h("div.hsearch", null, glyph(SEARCH_PATH, 13), search),
    chips,
    h("div.list.hlistbox", null, h("div.hlhead", null, h("span", null, "Rule"), h("span", null, "Violations")), rows, noMatch));
  const body = h("div.hist", null, aside,
    h("section", null, h("div.list", null, unmatched, detail, summaryLine, scroll)));
  pane = h("section.group.history", null,
    h("div.ghead.hh", null, h("h2", null, "History by rule"), lead, h("div.act", null, range)),
    empty, ltrow, body);
  pane._a = { lead, range, toggle, search, chips, rows, noMatch, unmatched, detail, scroll, title, pick, pickLabel, pickWrap, meta, kvs, summaryLine, violations, more, nothing, empty, ltrow, body };
}

const createListRow = () => h("button.hrow", { "data-act": "hsel" },
  h("span.l", null, h("span.t"), h("span.m")), h("span.c.num", null, h("b")));

function updateListRow(node, rule, selected) {
  setAttr(node, "data-i", rule.id);
  setAttr(node, "aria-current", String(rule.id === selected));
  const t = node.querySelector(".t");
  setText(t, rule.title);
  setAttr(t, "title", rule.title);
  setText(node.querySelector(".m"), rule.origin);
  setText(node.querySelector(".c b"), fmt(rule.count));
}

/** The list column, filtered by the search box. */
function renderList(a, ranked, id) {
  if (a.search.value !== query) a.search.value = query;
  const shown = filterRanked(ranked, query);
  syncChildren(a.rows, shown, r => r.id, createListRow, (node, r) => updateListRow(node, r, id));
  setAttr(a.noMatch, "hidden", shown.length > 0);
}

/** The rule's heading: its title, or with the list hidden a picker over every listed rule (and the shown one). */
function renderHeading(a, ranked, id) {
  const title = historyTitle(id);
  setText(a.title, title);
  setAttr(a.title, "title", title);
  setAttr(a.title, "hidden", listHidden);
  setAttr(a.pickWrap, "hidden", !listHidden);
  if (!listHidden) return;
  const options = ranked.some(r => r.id === id) ? ranked : [{ id, title }, ...ranked];
  syncChildren(a.pick, options, r => r.id, r => h("option", { value: r.id }), (o, r) => setText(o, r.title));
  if (a.pick.value !== id) a.pick.value = id;
  setText(a.pickLabel, title);
  setAttr(a.pick, "title", title);
}

/** What the coding agent is told on the rule's violations: the newest loaded one's message, else the rule's own. */
function toldOf(id) {
  const newest = data.findings.owner === id ? data.findings.rows[0]?.message : undefined;
  const stored = data.rules.get(id);
  return newest ?? (stored ? messageText(stored) : "");
}

/** The line under the title (where from · id · On or Off · Open rule), then the mode and what the agent is told. */
function renderMeta(a, id, told) {
  const stored = data.rules.get(id);
  const origin = historyOrigin(id);
  const mode = stored ? ruleMode(stored) : null;
  setContent(a.meta, JSON.stringify([id, stored?.enabled, origin]), () => [
    h("span", null, origin), h("span.mono", null, id),
    stored ? h("span", null, stored.enabled ? "On" : "Off") : h("span", null, "Deleted rule"),
    stored ? h("span.acts", null, h("button.btn.sm", { "data-act": "detail", "data-i": id }, "Open rule")) : null,
  ]);
  setAttr(a.kvs, "hidden", mode === null && told === "");
  setContent(a.kvs, JSON.stringify([mode, told]), () => [
    mode === null ? null : [h("dt", null, "On a violation"),
      h("dd", null, h(`span.oc.oc-${MODE_OUTCOME[mode] ?? "steered"}`, null, icon(MODE_OUTCOME[mode] ?? "steered", 14), MODE_LABEL[mode] ?? mode))],
    told === "" ? null : [h("dt", null, "What the agent sees"), h("dd", null, h("span.said2", { title: told }, h("span", null, told)))],
  ]);
}

const locationOf = x => (x.path ? `${x.path}${x.span ? `:${x.span.startLine + 1}` : ""}` : "");

function createViolation(item) {
  if (item.day !== undefined) return h("div.day", null, item.day);
  return h("div.ev.hr", null,
    h("span.when.num"),
    h("div.what", null, h("span.loc.mono"), h("small.diff", { hidden: true })),
    h("button.open", { "data-act": "session", title: "Open this session" }));
}

/** One violation: when (24-hour), where, its session; its message only where it differs from what the header says. */
function updateViolation(node, item, told) {
  if (item.day !== undefined) return;
  const x = item.x;
  setText(node.querySelector(".when"), CLOCK.format(x.at));
  const loc = node.querySelector(".loc");
  const where = locationOf(x);
  setText(loc, where || "No path recorded");
  loc.classList.toggle("muted", where === "");
  const diff = node.querySelector(".diff");
  const differs = Boolean(x.message) && x.message !== told;
  setAttr(diff, "hidden", !differs);
  setText(diff, differs ? x.message : "");
  const open = node.querySelector(".open");
  setAttr(open, "data-i", x.sessionId);
  setText(open, shortSessionId(x.sessionId, x.host));
}

/** The shown rule's loaded violations, by day; empty while its first page is still to come. */
function violationItems(id) {
  const slot = data.findings;
  if (slot.owner !== id) return [];
  const items = [];
  let lastDay = null;
  for (const x of slot.rows) {
    const day = dayLabel(daysAgo(x.at));
    if (day !== lastDay) { items.push({ key: `day:${day}`, day }); lastDay = day; }
    items.push({ key: `${x.auditId}:${x.id}`, x });
  }
  return items;
}

function renderViolations(a, id, count, told) {
  const slot = data.findings, loaded = slot.owner === id && slot.pages > 0, label = rangeLabel();
  const total = loaded ? slot.total : count;
  const activity = data.activity[id];
  const sessions = activity === undefined || activity.measured === false ? 0 : activity.sessions;
  setContent(a.summaryLine.firstChild, JSON.stringify([total, label, sessions]), () => [
    h("b", null, fmt(total)), ` ${plural(total, "violation")} logged ${label}`,
    sessions > 0 ? ` · ${fmt(sessions)} ${plural(sessions, "session")}` : null,
  ]);
  const none = loaded && slot.rows.length === 0;
  setAttr(a.nothing, "hidden", !none);
  if (none) setText(a.nothing.querySelector("p"), `${historyTitle(id)} logged no violations ${label}. Widen the range above to see earlier ones.`);
  syncChildren(a.violations, violationItems(id), i => i.key, createViolation, (node, item) => updateViolation(node, item, told));
  const left = loaded && slot.next !== null ? slot.total - slot.rows.length : 0;
  setAttr(a.more, "hidden", left <= 0);
  if (left > 0) setText(a.more.firstChild, `Load more · ${fmt(left)} left`);
}

const EMPTY_TEXT = {
  noRules: ["No history yet", () => "Add a rule pack or create a rule, then run a coding agent session. Violations show here, grouped by rule."],
  noSessions: ["No history yet", () => "Violations show here, grouped by rule, once a rule logs one in a coding agent session."],
  noViolations: ["No violations yet", e => `Rerail audited ${fmt(e.sessions)} ${plural(e.sessions, "session")} in this repo and logged none. When a rule logs one, it shows here grouped by rule.`],
  nothingInRange: ["Nothing in this range", () => `No rule logged a violation ${rangeLabel()}. Widen the range above to see earlier ones.`],
};

/** The line under the title: how many rules the list holds, and under a chip how many violations they count. */
function leadText(ranked) {
  const label = rangeLabel(), n = ranked.length, rules = `${fmt(n)} ${plural(n, "rule")}`;
  if (ui.histOutcome === "all") return `${rules} with violations ${label}, most first`;
  const total = ranked.reduce((sum, r) => sum + r.count, 0), violations = `${fmt(total)} ${plural(total, "violation")}`;
  return ui.histOutcome === "steered" ? `${rules} · ${violations} steered ${label}, most first` : `${rules} · ${violations} logged on Log ${label}, most first`;
}

/** A chip whose list is empty, while other violations are in range: what the section says in place of a rule. */
function renderUnmatched(a, show) {
  setAttr(a.unmatched, "hidden", !show);
  for (const part of [a.detail, a.summaryLine, a.scroll]) setAttr(part, "hidden", show);
  if (!show) return;
  const steered = ui.histOutcome === "steered";
  setText(a.unmatched.querySelector("h3"), steered ? `Nothing steered ${rangeLabel()}` : `Nothing logged on Log ${rangeLabel()}`);
  setText(a.unmatched.querySelector("p"), steered
    ? `Every violation ${rangeLabel()} was logged by a rule on Log, which records without telling the coding agent.`
    : `Every violation ${rangeLabel()} was steered: the rules that logged them were on Steer.`);
}

export function render(root) {
  if (pane === null) build();
  if (pane.parentNode !== root) root.replaceChildren(pane);
  const a = pane._a;
  syncRange(a.range);
  const every = rankedRules("all");
  const ranked = ui.histOutcome === "all" ? every : rankedRules();
  const empty = historyEmpty(every);
  const id = historyRuleId(ranked);
  const showBody = empty === null || (id !== null && ui.actRule !== null);
  setText(a.lead, empty === null ? leadText(ranked) : "");
  // The range only matters once something was logged: before, every range reads the same.
  setAttr(a.range, "hidden", empty !== null && empty.kind !== "nothingInRange" && !showBody);
  // Until the activity answers are in (the range's, and since install's for the empty states), nothing is known to be empty.
  const known = data.activitySessions !== null && (ui.range === 9999 || data.activityAllSessions !== null);
  setAttr(a.empty, "hidden", showBody || empty === null || !known);
  if (!showBody && empty !== null && known) {
    const [title, text] = EMPTY_TEXT[empty.kind];
    setText(a.empty.querySelector("h3"), title);
    setText(a.empty.querySelector("p"), text(empty));
  }
  setAttr(a.ltrow, "hidden", !showBody);
  setAttr(a.body, "hidden", !showBody);
  if (!showBody) return;
  a.body.classList.toggle("nolist", listHidden);
  const label = listHidden ? "Show list" : "Hide list";
  setAttr(a.toggle, "aria-pressed", String(!listHidden));
  setAttr(a.toggle, "aria-label", label);
  setText(a.toggle.lastChild, label);
  for (const c of a.chips.children) setAttr(c, "aria-pressed", String(c.dataset.i === ui.histOutcome));
  renderList(a, ranked, id);
  renderUnmatched(a, id === null);
  if (id === null) return;
  renderHeading(a, ranked, id);
  const told = toldOf(id);
  renderMeta(a, id, told);
  renderViolations(a, id, inRange(id), told);
}

/**
 * Entered from a link or the menu: the route's rule (`?rule=<id>`, a "See all", a steer link), else the top of the list;
 * the route's chip (`?outcome=`), else All.
 */
export function enter(route) {
  ui.actRule = route.filterRule ?? null;
  ui.histOutcome = route.outcome ?? "all";
}

/** The shown rule follows the ranking (the top one, before any is chosen): fetch its violations once it changes. */
export function afterPoll() {
  const id = historyRuleId();
  if (id !== null && data.findings.owner !== id) refreshNow();
}

/** Show rule `id` and fetch its violations. */
function selectRule(id) {
  ui.actRule = id;
  invalidate();
  refreshNow();
}

export const actions = {
  hsel: id => selectRule(id),
  hmore: () => loadMore(loadMoreFindings),
  hlisttoggle: () => { listHidden = !listHidden; invalidate(); },
  // A chip starts over at the top of its own list.
  houtcome: outcome => { ui.histOutcome = outcome; ui.actRule = null; invalidate(); refreshNow(); },
};

export function onInput(el) {
  if (el.id === "hq") { query = el.value; invalidate(); }
}

export function onChange(el) {
  if (el.id === "hpick") selectRule(el.value);
}
