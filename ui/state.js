/**
 * One store for fetched data and one for view state. Render modules read both; user actions (the shell, `actions.js`,
 * `sheet.js`, the frame and each view's own actions) write `ui` and optimistic `data`, and `poll.js` writes fetches.
 * Rules are keyed by id so a poll can patch the row that changed and nothing else.
 */
import { isFromPack, isRunning, packOriginName, ruleTitle, rulePrefix, scopeText } from "./derive.js";
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

/** Show `scope`. A worktree in it narrows what the page reads; every change still belongs to the repository. */
export function setScope(scope) {
  ui.scope = scope;
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
  /** History: the shown rule's violations in range, newest first, the pages loaded so far (`owner`: the rule id). */
  findings: pagedList(),
  /** The open rule panel's violations in range (`owner`: the rule id). */
  ruleFindings: pagedList(),
  /** Sessions view: session summaries, newest first, the pages loaded so far. */
  sessions: pagedList(),
  /** The selected session's violations, all time (`owner`: the session id). */
  sessionFindings: pagedList(),
  /** Overview's Recent sessions: the range's five newest sessions (`rows`, rows of GET /api/sessions) and `total` started in range; null until loaded. */
  recentSessions: null,
  /**
   * sessionId → { head, events, nextAfter, loaded, earlier, loading }: `head` is the session's summary from
   * `GET /api/sessions/{id}`, `events` the transcript loaded so far (the newest page first), `earlier` while older ones may be left.
   */
  sessionDetail: new Map(),
  /** sessionId → first useful user prompt (null while unknown). */
  titles: new Map(),
  /** Every rule pack installed on this machine (`GET /api/rule-packs`), languages first, then frameworks, and the scoped checkout's own rules. */
  packs: [],
  /** The installed packs left out, and the scoped checkout's `.rerail/rules` files left out, with why (`GET /api/rule-packs` `skipped`). */
  packsSkipped: [],
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
  /** The daemon answered `401`: this browser has not opened the dashboard link, so it holds no capability. */
  accessRequired: false,
  loaded: false,
};

export const RANGES = [[0, "Today"], [7, "7 days"], [30, "30 days"], [9999, "Since install"]];

export const ui = {
  /** Set by `setScope` before the first poll; `null` lists repositories. */
  scope: null,
  settingUp: false,
  /** The open view (`routes.js`), and the pack the `pack` view shows. */
  view: "overview",
  pack: null,
  range: 7,
  filter: "all",
  query: "",
  sort: "violations",
  /** The sheet: `{ kind: "rule", id }`, `{ kind: "pack", id }`, or null. */
  panel: null,
  /** Where Back goes from a rule opened inside a pack's panel. */
  backToPack: null,
  askOpen: false,
  askIntent: "ask",
  /** History: the rule chosen (a `?rule=` link or a click); null shows the top-ranked one. */
  actRule: null,
  /** History by rule's chip ("all", "steered", "logged"): which violations its list counts (`?outcome=`). */
  histOutcome: "all",
  /** Rule packs page: list or cards, and the pack whose Remove waits for its second click. */
  catView: "list",
  removeAsk: null,
  /** Packs removed from the catalog during this visit: their row reads Removed with Undo until the page is left. */
  removedPacks: new Set(),
  /** Overview's day-one banner, dismissed for this repository. */
  welcomeDismissed: false,
  selectedSession: null,
  sessQuery: "",
  sessFilter: "all",
  /** History by session: the one rule the selected session's timeline is filtered to, or null for every rule. */
  sessRule: null,
  showAll: false,
  openDays: new Set(["d0"]),
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
/** The rule pack a pack's rule ships in, whether or not that pack is added here. */
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
  if (f === "on" && !isRunning(stored)) return false;
  if (f === "off" && isRunning(stored)) return false;
  if (f === "fired" && fires(id) === 0) return false;
  if (ui.query && !searchText(stored).includes(ui.query)) return false;
  return true;
}
export const filtering = () => ui.filter !== "all" || ui.query !== "";

/**
 * The rule list's sections, in order: Created rules, one per rule pack added here, then every other
 * rule from a pack (added on its own, not through it). Each section holds its groups; a group holds
 * every rule (`all`).
 */
export function ruleSections() {
  const mine = [], byPack = new Map(), other = new Map();
  for (const r of data.rules.values()) {
    if (!isFromPack(r)) { mine.push(r); continue; }
    const pack = attachedPackOf(r);
    if (pack === undefined) {
      const known = packOfRule(r);
      const key = known === undefined ? rulePrefix(r.rule.id) : packOriginName(known);
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
  const group = (name, all) => ({ name, all });
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

/** Every violation of the repository's rules in range: what a pack page's share of violations is of. */
export const totalFires = () => rulesArray().reduce((sum, r) => sum + fires(r.rule.id), 0);

/** The open rule panel's violations in range, newest first, from the pages loaded for it. */
export const ruleFindings = id => (data.ruleFindings.owner === id ? data.ruleFindings.rows : []);
/** Every violation of the rule in range, once its first page is in; 0 before. */
export const ruleFindingsTotal = id => (data.ruleFindings.owner === id ? data.ruleFindings.total : 0);

/** The rule a panel shows, when the panel shows one. */
export const panelRuleId = () => (ui.panel?.kind === "rule" ? ui.panel.id : null);
