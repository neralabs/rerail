/**
 * One store for fetched data and one for view state. Render modules read both; only
 * `app.js` (user actions) and `poll.js` (fetches) write. Rules are keyed by id so a poll can
 * patch the row that changed and nothing else.
 */
import { isLog, isPreset, searchText, ruleTitle, isNew, originOf, rulePrefix } from "./derive.js";
import { startOfDay } from "./format.js";

export const data = {
  repo: "",
  /** id → stored rule, insertion order = catalog order. */
  rules: new Map(),
  /** id → activity for the selected range, and for all time (`since=0`). */
  activity: {},
  activityAll: {},
  /** Newest first, the pages fetched so far for the selected range. */
  findings: [],
  findingsTotal: 0,
  /** Session summaries, newest first. */
  sessions: [],
  /** sessionId → { events: [], nextAfter, loaded, loading } */
  sessionDetail: new Map(),
  /** sessionId → first useful user prompt (null while unknown). */
  titles: new Map(),
  /** Freshness: last successful poll, and whether the last attempt failed. */
  updatedAt: 0,
  disconnected: false,
  loaded: false,
};

export const RANGES = [[0, "Today"], [7, "7 days"], [30, "30 days"], [9999, "Since install"]];

export const ui = {
  view: "rules",
  range: 7,
  filter: "all",
  query: "",
  sort: "triggered",
  groupBy: "origin",
  tab: "rules",
  openRows: new Set(),
  selectedRule: null,
  askOpen: false,
  askIntent: "ask",
  recentShown: 50,
  selectedSession: null,
  sessQuery: "",
  sessFilter: "all",
  evFilter: "all",
  showAll: false,
  openDays: new Set(["d0"]),
  dayShown: {},
  openEvents: new Set(),
  closedEvents: new Set(),
  openFolds: new Set(),
  resumeOpen: false,
  theme: "",
};

/**
 * `since` for the current range: local midnight for "today", 0 for "since install". Sliding
 * ranges are quantized to ten minutes so the poll URL — and therefore its ETag — stays the same
 * between polls instead of shifting by the milliseconds elapsed.
 */
const QUANTUM = 600_000;
export function sinceFor(range = ui.range) {
  if (range === 0) return startOfDay();
  if (range >= 9999) return 0;
  return Math.floor((Date.now() - range * 86_400_000) / QUANTUM) * QUANTUM;
}

const RANGE_LABEL = { 0: "today", 7: "last 7 days", 30: "last 30 days" };
export const rangeLabel = () => RANGE_LABEL[ui.range] ?? "since install";

const NONE = { measured: true, distinctViolations: 0, sessions: 0, lastSeenAt: null, evaluations: 0, rerails: 0 };
export const activityOf = id => data.activity[id] ?? NONE;
export const activityAllOf = id => data.activityAll[id] ?? NONE;
export const isMeasured = id => activityOf(id).measured !== false;
/** The number a row shows: distinct violations in range (Q20b). Cosine rules read 0 (Q22a). */
export const fires = id => (isMeasured(id) ? activityOf(id).distinctViolations : 0);
export const allFires = id => (isMeasured(id) ? activityAllOf(id).distinctViolations : 0);
export const rerails = id => activityOf(id).rerails;

export const rulesArray = () => [...data.rules.values()];

export function matches(stored) {
  const id = stored.rule.id, origin = stored.catalogOrigin;
  const f = ui.filter;
  if (f === "added" && origin !== "local") return false;
  if (f === "preset" && origin !== "bundled") return false;
  if (f === "on" && !stored.enabled) return false;
  if (f === "off" && stored.enabled) return false;
  if (f === "fired" && fires(id) === 0) return false;
  if (ui.query && !searchText(stored).includes(ui.query)) return false;
  return true;
}

const az = (a, b) => ruleTitle(a).localeCompare(ruleTitle(b));
const newest = (a, b) => b.createdAt - a.createdAt;

export function sortRules(list) {
  const c = [...list];
  switch (ui.sort) {
    case "az": return c.sort(az);
    case "newest": return c.sort((a, b) => newest(a, b) || az(a, b));
    case "recent": return c.sort((a, b) => (activityOf(b.rule.id).lastSeenAt ?? 0) - (activityOf(a.rule.id).lastSeenAt ?? 0) || az(a, b));
    default: return c.sort((a, b) => fires(b.rule.id) - fires(a.rule.id) || (isNew(b) ? 1 : 0) - (isNew(a) ? 1 : 0) || az(a, b));
  }
}

/** Every per-render aggregate, computed once. */
export function summary() {
  const rules = rulesArray();
  const on = rules.filter(r => r.enabled);
  let triggers = 0, rerailTotal = 0, steerRules = 0, firedN = 0, firedOn = 0;
  const sessionLower = new Set();
  let maxRuleSessions = 0;
  for (const r of rules) {
    const id = r.rule.id, f = fires(id), a = activityOf(id);
    triggers += f;
    rerailTotal += a.rerails;
    if (a.rerails > 0) steerRules += 1;
    if (f > 0) { firedN += 1; if (r.enabled) firedOn += 1; }
    if (a.sessions > maxRuleSessions) maxRuleSessions = a.sessions;
  }
  const since = sinceFor();
  for (const x of data.findings) if (x.at >= since) sessionLower.add(x.sessionId);
  const sessionsInRange = data.sessions.filter(s => s.startedAt >= since);
  return {
    rules, on,
    presetsOff: rules.filter(r => isPreset(r) && !r.enabled).length,
    neverTriggered: on.filter(r => isMeasured(r.rule.id) && allFires(r.rule.id) === 0).length,
    triggers, rerails: rerailTotal, steerRules, flagged: Math.max(0, triggers - rerailTotal),
    firedN, firedOn,
    sessionsInRange: sessionsInRange.length,
    // Lower bound: distinct sessions across fetched findings, or the busiest rule's own count.
    sessionsWithTriggers: Math.max(sessionLower.size, maxRuleSessions),
  };
}

/** Findings in range for one rule, newest first, from the fetched pages. */
export const ruleFindings = id => data.findings.filter(x => x.ruleId === id);

export const originGroups = () => {
  const mine = [], presets = [];
  for (const r of data.rules.values()) if (matches(r)) (isPreset(r) ? presets : mine).push(r);
  return { mine: sortRules(mine), presets: sortRules(presets) };
};

export function prefixGroups() {
  const groups = new Map();
  for (const r of data.rules.values()) {
    const key = rulePrefix(r.rule.id);
    let g = groups.get(key);
    if (g === undefined) { g = { key, all: [], shown: [], onN: 0, act: 0 }; groups.set(key, g); }
    g.all.push(r);
    if (r.enabled) g.onN += 1;
    g.act += fires(r.rule.id);
    if (matches(r)) g.shown.push(r);
  }
  return [...groups.values()].filter(g => g.shown.length)
    .map(g => ({ ...g, shown: sortRules(g.shown) }))
    .sort((a, b) => b.act - a.act || a.key.localeCompare(b.key));
}

export const originLabel = stored => originOf(stored).label;
export { isLog };
