/**
 * One store for fetched data and one for view state. Render modules read both; only
 * `app.js` (user actions) and `poll.js` (fetches) write. Rules are keyed by id so a poll can
 * patch the row that changed and nothing else.
 */
import { isLog, isByRerail, isRunning, ruleTitle, rulePrefix, scopeText, categoryLabel } from "./derive.js";
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

/**
 * A list the daemon pages with a cursor: `key` is its URL without the cursor (a new key starts over), `owner` what it
 * lists (a rule or session id, when it lists one), `rows` the loaded pages newest first, `total` every matching row,
 * `next` the cursor past the last loaded row (`null`: nothing further), `pages` how many pages are loaded (0: none yet).
 */
export const pagedList = (key = null, owner = null) => ({ key, owner, rows: [], total: 0, next: null, pages: 0 });

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
  /** `{ started, withViolations }`: sessions in the range, and since install, from the activity answers. */
  activitySessions: null,
  activityAllSessions: null,
  /** Activity: the range's violations, newest first, the pages loaded so far (`pagedList`). */
  findings: pagedList(),
  /** The open rule panel's violations in range (`owner`: the rule id). */
  ruleFindings: pagedList(),
  /** Sessions view: session summaries, newest first, the pages loaded so far. */
  sessions: pagedList(),
  /** The selected session's violations, all time (`owner`: the session id). */
  sessionFindings: pagedList(),
  /** The newest session of the repository, or `null`: the welcome banner. */
  newestSession: null,
  /**
   * sessionId → { head, events, nextAfter, loaded, earlier, loading }: `head` is the session's summary from
   * `GET /api/sessions/{id}`, `events` the transcript loaded so far (the newest page first), `earlier` while older ones may be left.
   */
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
  /** The scoped repository's Follow .gitignore setting (`repository_context_get`); loaded for Settings, `null` until then. */
  followGitignore: null,
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

const NONE = { measured: true, distinctViolations: 0, sessions: 0, lastSeenAt: null, audits: 0, loggedViolations: 0, rerails: 0 };
export const activityOf = id => data.activity[id] ?? NONE;
export const activityAllOf = id => data.activityAll[id] ?? NONE;
export const isMeasured = id => activityOf(id).measured !== false;
/** The number a row shows: distinct violations in range (Q20b). Cosine rules read 0 (Q22a). */
export const fires = id => (isMeasured(id) ? activityOf(id).distinctViolations : 0);
export const allFires = id => (isMeasured(id) ? activityAllOf(id).distinctViolations : 0);
/** Whether any audit since install evaluated or flagged the rule: what makes a zero mean "clean" rather than "never run". */
export const isAudited = id => isMeasured(id) && (activityAllOf(id).audits > 0 || activityAllOf(id).distinctViolations > 0);
export const rerails = id => activityOf(id).rerails;

export const rulesArray = () => [...data.rules.values()];

/* ---------- the scoped repository ---------- */

/** The scoped repository's name, once its context has loaded. */
export const repoName = () => data.repository?.name ?? "";
/** Rerail's state where the page looks: the checkout's effective state, else the repository's own. */
export const stateHere = () => data.checkout?.state ?? data.repository?.state;
/** The repository has a configuration: something was attached, or it was set up with none. */
export const isSetUp = () => data.repository?.setUp === true;
/**
 * The packs kept on: the packs attached to a repository that is set up; before it is, the ones set-up
 * recommends, until the first switch writes the kept set and sets the repository up.
 */
export function keptPacks() {
  if (isSetUp()) return [...data.attachedPacks.keys()];
  return (data.recommendations ?? []).map(r => r.packId);
}
/** The review of a repository that is not set up yet: a pack is on there when it is kept, and changes write the kept set. */
export const reviewing = () => ui.view === "review" && !isSetUp();

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
  if (f === "on" && !isRunning(stored)) return false;
  if (f === "off" && isRunning(stored)) return false;
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

/** How many of the repository's rules run: switched on, with options that apply. */
export const runningCount = () => [...data.rules.values()].filter(isRunning).length;

/** Every per-render aggregate, computed once. */
export function summary() {
  const rules = rulesArray();
  const on = rules.filter(isRunning);
  let triggers = 0, rerailTotal = 0, steerRules = 0, firedN = 0, firedOn = 0, logged = 0;
  for (const r of rules) {
    const id = r.rule.id, f = fires(id), a = activityOf(id);
    triggers += f;
    rerailTotal += a.rerails;
    // Counted by the mode recorded with each violation; the rule may have changed mode since.
    if (isMeasured(id)) logged += a.loggedViolations ?? 0;
    if (a.rerails > 0) steerRules += 1;
    if (f > 0) { firedN += 1; if (isRunning(r)) firedOn += 1; }
  }
  const logOn = on.filter(isLog).length;
  return {
    rules, on, logOn, steerOn: on.length - logOn,
    triggers, rerails: rerailTotal, steerRules, logged,
    firedN, firedOn,
    // Exact, from the activity answer: sessions started in range, and sessions with a violation logged in range.
    sessionsInRange: data.activitySessions?.started ?? 0,
    sessionsWithTriggers: data.activitySessions?.withViolations ?? 0,
    // Whether any session was ever recorded here: the overview's "your next session shows here".
    anySessions: (data.activityAllSessions?.started ?? 0) > 0,
  };
}

/** Activity's fallback pack name for a By Rerail rule not added through a pack. */
const OTHER_RULES = "Other rules";
/** The rule pack a violation files under on Activity: the added pack its rule is present through, else Other or Created rules. */
export const activityPackOf = stored => (stored ? attachedPackOf(stored)?.name ?? (isByRerail(stored) ? OTHER_RULES : "Created rules") : OTHER_RULES);

/**
 * What Activity shows under its pack and rule filters. The loaded pages are a prefix of the range, so
 * while the server has more (`findings.next`) a filter's matches may lie further back: `more` stays
 * available and an empty filtered list is not yet "none".
 */
export function activityView() {
  const findings = data.findings.rows;
  const filtered = ui.actPack !== "" || ui.actRule !== null;
  const list = filtered
    ? findings.filter(x => (!ui.actPack || activityPackOf(data.rules.get(x.ruleId)) === ui.actPack) && (ui.actRule === null || x.ruleId === ui.actRule))
    : findings;
  const serverHasMore = data.findings.next !== null;
  const shown = list.slice(0, ui.recentShown);
  const loadedLeft = list.length - shown.length;
  // Unfiltered, the server's total is the exact count; filtered, only the loaded matches are known.
  const total = filtered ? list.length : Math.max(data.findings.total, list.length);
  return {
    filtered, list, shown,
    none: list.length === 0 && !(filtered && serverHasMore),
    more: filtered
      ? (loadedLeft > 0 || serverHasMore) && { count: loadedLeft > 0 ? Math.min(50, loadedLeft) : null, of: serverHasMore ? null : total }
      : total - shown.length > 0 && { count: Math.min(50, total - shown.length), of: total },
  };
}

/** The open rule panel's violations in range, newest first, from the pages loaded for it. */
export const ruleFindings = id => (data.ruleFindings.owner === id ? data.ruleFindings.rows : []);
/** Every violation of the rule in range, once its first page is in; 0 before. */
export const ruleFindingsTotal = id => (data.ruleFindings.owner === id ? data.ruleFindings.total : 0);

/** The rule a panel shows, when the panel shows one. */
export const panelRuleId = () => (ui.panel?.kind === "rule" ? ui.panel.id : null);
