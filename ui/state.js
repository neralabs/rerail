/**
 * One store for fetched data and one for view state. Render modules read both; only
 * `app.js` (user actions) and `poll.js` (fetches) write. Rules are keyed by id so a poll can
 * patch the row that changed and nothing else.
 */
import { isLog, isByRerail, ruleTitle, rulePrefix, scopeText, categoryLabel } from "./derive.js";
import { startOfDay } from "./format.js";

/**
 * The repository the page shows, from `?repository=<key>&checkout=<path>` (either names it; the checkout also
 * names the worktree whose own settings the page can change). `null` without either: the page lists repositories.
 */
export function readScope(search) {
  const params = new URLSearchParams(search);
  const repository = params.get("repository") || undefined;
  const checkout = params.get("checkout") || undefined;
  return repository === undefined && checkout === undefined ? null : { repository, checkout };
}

/** Show `scope`; a change made on the page belongs to its worktree when the scope names one, else to the repository. */
export function setScope(scope) {
  ui.scope = scope;
  ui.owner = scope?.checkout === undefined ? "repository" : "checkout";
}

export const data = {
  /** The scoped repository (`repository_context_get`): name, key, set up, started; `null` until loaded. */
  repository: null,
  /** The scoped checkout as the daemon resolved it (`{checkoutPath, folder, state}`), or `null`. */
  checkout: null,
  /** Every repository Rerail knows (`GET /api/repositories`): the picker and Settings. */
  repositories: [],
  /** Attachments whose rule or pack cannot be found. */
  unavailable: [],
  /** id → stored rule, insertion order = catalog order. */
  rules: new Map(),
  /**
   * Pack id → the folders it is attached at in the scoped repository: the packs added here. A pack shown added by a
   * write still waiting for the recommendations has a promise of its folder until they arrive or the poll reads it back.
   */
  attachedPacks: new Map(),
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
  /** Every rule pack that comes with Rerail (`GET /api/rule-packs`), languages first, then frameworks. */
  packs: [],
  /** `/healthz`: version, database path; `null` until the daemon answers once. */
  health: null,
  /** Coding agents found and connected (`GET /api/coding-agents`); loaded for Settings. */
  agents: null,
  /** The database-wide status line preference: "compact" (Show) or "silent" (Do not show). */
  presence: null,
  /** Recommended packs for the scoped repository, each with its folder; fetched only when needed. */
  recommendations: null,
  /** Why the coding agents could not be checked, when `GET /api/coding-agents` failed. */
  agentsError: null,
  /** Freshness: last successful poll, and whether the last attempt failed. */
  updatedAt: 0,
  disconnected: false,
  /** The daemon answered that it does not know the scoped repository or checkout. */
  missing: false,
  loaded: false,
};

export const RANGES = [[0, "Today"], [7, "7 days"], [30, "30 days"], [9999, "Since install"]];

export const ui = {
  /** Set by `setScope` before the first poll; `null` lists repositories. */
  scope: null,
  /** Who a rule change belongs to: this worktree or the repository. */
  owner: "repository",
  settingUp: false,
  view: "rules",
  range: 7,
  filter: "all",
  query: "",
  sort: "violations",
  /** The side panel: `{ kind: "rule", id }`, `{ kind: "pack", id }`, `{ kind: "create" }`, or null. */
  panel: null,
  /** Where Back goes from a rule opened inside a pack's panel. */
  backToPack: null,
  askOpen: false,
  askIntent: "ask",
  recentShown: 50,
  /** Activity filters: one rule pack, then one rule. */
  actPack: "",
  actRule: null,
  /** Rules view folds: sections hidden, and groups folded ("section:group" → open?). */
  hiddenSections: new Set(),
  groupOpen: new Map(),
  foldedSections: new Set(),
  /** Rule packs page: list or cards, and the pack whose Remove waits for its second click. */
  catView: "list",
  removeAsk: null,
  /** W0: "loading" while recommendations load, "shown" once asked, "dismissed" after Not now. */
  recommend: "idle",
  welcomeDismissed: false,
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
  wide: false,
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

const NONE = { measured: true, distinctViolations: 0, sessions: 0, lastSeenAt: null, audits: 0, rerails: 0 };
export const activityOf = id => data.activity[id] ?? NONE;
export const activityAllOf = id => data.activityAll[id] ?? NONE;
export const isMeasured = id => activityOf(id).measured !== false;
/** The number a row shows: distinct violations in range (Q20b). Cosine rules read 0 (Q22a). */
export const fires = id => (isMeasured(id) ? activityOf(id).distinctViolations : 0);
export const allFires = id => (isMeasured(id) ? activityAllOf(id).distinctViolations : 0);
export const rerails = id => activityOf(id).rerails;

export const rulesArray = () => [...data.rules.values()];

/* ---------- the scoped repository ---------- */

/** The scoped repository's name, once its context has loaded. */
export const repoName = () => data.repository?.name ?? "";
/** Rerail's state where the page looks: the checkout's effective state, else the repository's own. */
export const stateHere = () => data.checkout?.state ?? data.repository?.state;
/** The repository has a configuration: something was attached, or it was set up with none. */
export const isSetUp = () => data.repository?.setUp === true;

/* ---------- rule packs ---------- */

let packIndex = { packs: null, byId: new Map(), rules: new Map() };
/** Pack id → pack, and rule id → `{ pack, rule }` from the pack data, rebuilt when the packs change. */
function packs() {
  if (packIndex.packs !== data.packs) {
    const byId = new Map(), rules = new Map();
    for (const pack of data.packs) {
      byId.set(pack.id, pack);
      for (const rule of pack.rules) rules.set(rule.id, { pack, rule });
    }
    packIndex = { packs: data.packs, byId, rules };
  }
  return packIndex;
}
export const packById = id => packs().byId.get(id);
/** The pack's own entry for a rule: title, group and its caught and passing examples. */
export const packRuleOf = id => packs().rules.get(id);
/** The rule pack a By Rerail rule ships in, whether or not that pack is added here. */
export const packOfRule = stored => packRuleOf(stored.rule.id)?.pack;
/** The added pack a rule is present through; `undefined` for a rule added on its own. */
export const attachedPackOf = stored => (stored.source?.kind === "pack" ? packById(stored.source.id) : undefined);
/** A pack is added here when the repository has an attachment for it, at any folder. */
export const isAttached = pack => data.attachedPacks.has(pack.id);
export const packGroups = pack => [...new Set(pack.rules.map(r => r.group))];
/** The folder set-up adds a pack in: its folder in `recommendations` (fetched for the write's scope), else the whole repository. */
export const packFolder = (recommendations, id) => recommendations.find(r => r.packId === id)?.folder ?? "";

/** The row title: the pack's plain title when the rule comes from one, else the rule's own. */
export const titleOf = stored => packRuleOf(stored.rule.id)?.rule.title ?? ruleTitle(stored);
/** "By Rerail · <the pack's category>", "By Rerail", or "Created by you". */
export function originChip(stored) {
  if (!isByRerail(stored)) return "Created by you";
  const category = categoryLabel(packOfRule(stored)?.category);
  return category ? `By Rerail · ${category}` : "By Rerail";
}

const haystacks = new WeakMap();
/** Text the search box runs over: title, id, description, scope, pack and group. */
function searchText(stored) {
  let hay = haystacks.get(stored);
  if (hay === undefined) {
    const entry = packRuleOf(stored.rule.id);
    hay = [titleOf(stored), stored.rule.id, stored.rule.description ?? "", scopeText(stored), entry?.pack.name ?? "", entry?.rule.group ?? ""].join(" ").toLowerCase();
    haystacks.set(stored, hay);
  }
  return hay;
}

export function matches(stored) {
  const id = stored.rule.id;
  const f = ui.filter;
  if (f === "mine" && isByRerail(stored)) return false;
  if (f === "rerail" && !isByRerail(stored)) return false;
  if (f === "on" && !stored.enabled) return false;
  if (f === "off" && stored.enabled) return false;
  if (f === "fired" && fires(id) === 0) return false;
  if (ui.query && !searchText(stored).includes(ui.query)) return false;
  return true;
}
export const filtering = () => ui.filter !== "all" || ui.query !== "";

const az = (a, b) => titleOf(a).localeCompare(titleOf(b));

export function sortRules(list) {
  const c = [...list];
  switch (ui.sort) {
    case "az": return c.sort(az);
    case "recent": return c.sort((a, b) => (activityOf(b.rule.id).lastSeenAt ?? 0) - (activityOf(a.rule.id).lastSeenAt ?? 0) || az(a, b));
    default: return c.sort((a, b) => fires(b.rule.id) - fires(a.rule.id) || az(a, b));
  }
}

/**
 * The Rules view's sections, in order: Created rules, one per rule pack added here, then every other
 * rule by Rerail (added on its own, not through a pack). Each section holds its groups; a group holds
 * every rule (`all`) and the ones the filters let through (`shown`).
 */
export function ruleSections() {
  const mine = [], byPack = new Map(), other = new Map();
  for (const r of data.rules.values()) {
    if (!isByRerail(r)) { mine.push(r); continue; }
    const pack = attachedPackOf(r);
    if (pack === undefined) {
      const key = packOfRule(r)?.name ?? rulePrefix(r.rule.id);
      if (!other.has(key)) other.set(key, []);
      other.get(key).push(r);
      continue;
    }
    if (!byPack.has(pack.id)) byPack.set(pack.id, new Map(packGroups(pack).map(g => [g, []])));
    const group = packRuleOf(r.rule.id)?.rule.group ?? "Other";
    const groups = byPack.get(pack.id);
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group).push(r);
  }
  const group = (name, all) => ({ name, all, shown: sortRules(all.filter(matches)) });
  const sections = [{ key: "mine", kind: "mine", groups: [group("Created by you", mine)] }];
  for (const pack of data.packs) {
    const groups = byPack.get(pack.id);
    if (groups === undefined) continue;
    sections.push({ key: `pack:${pack.id}`, kind: "pack", pack, groups: [...groups].filter(([, all]) => all.length).map(([g, all]) => group(g, all)) });
  }
  if (other.size) {
    sections.push({ key: "legacy", kind: "legacy", groups: [...other].sort(([a], [b]) => a.localeCompare(b)).map(([g, all]) => group(g, all)) });
  }
  return sections;
}

/** Every per-render aggregate, computed once. */
export function summary() {
  const rules = rulesArray();
  const on = rules.filter(r => r.enabled);
  let triggers = 0, rerailTotal = 0, steerRules = 0, firedN = 0, firedOn = 0, logged = 0;
  let maxRuleSessions = 0;
  for (const r of rules) {
    const id = r.rule.id, f = fires(id), a = activityOf(id);
    triggers += f;
    rerailTotal += a.rerails;
    if (isLog(r)) logged += f;
    if (a.rerails > 0) steerRules += 1;
    if (f > 0) { firedN += 1; if (r.enabled) firedOn += 1; }
    if (a.sessions > maxRuleSessions) maxRuleSessions = a.sessions;
  }
  const since = sinceFor();
  const sessionLower = new Set();
  for (const x of data.findings) if (x.at >= since) sessionLower.add(x.sessionId);
  const sessionsInRange = data.sessions.filter(s => s.startedAt >= since);
  const logOn = on.filter(isLog).length;
  return {
    rules, on, logOn, steerOn: on.length - logOn,
    triggers, rerails: rerailTotal, steerRules, logged,
    firedN, firedOn,
    sessionsInRange: sessionsInRange.length,
    // Lower bound: distinct sessions across fetched findings, or the busiest rule's own count.
    sessionsWithTriggers: Math.max(sessionLower.size, maxRuleSessions),
  };
}

/** Findings in range for one rule, newest first, from the fetched pages. */
export const ruleFindings = id => data.findings.filter(x => x.ruleId === id);

/** The rule a panel shows, when the panel shows one. */
export const panelRuleId = () => (ui.panel?.kind === "rule" ? ui.panel.id : null);
