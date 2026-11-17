/**
 * Fetch orchestration: one conditional GET per resource every 5 s (`If-None-Match`; 304 means
 * nothing changed), backing off to 30 s while the daemon is unreachable, and paused while the tab is hidden.
 * Data lands in the store; the caller re-renders and reconciles in place. Every poll reads the repository
 * list and the daemon's health, and with a repository in scope its context; beyond that only the needs the
 * page names (`setNeeds`: the open view's, the sidebar's and an open sheet's, from `views/index.js`). Long lists
 * are paged with a cursor (`paged-lists.js`), a selected session loads through `session-detail.js` and Merge
 * Assist through `merge-assist.js`; what a request needs to land in the right scope is in `scoped-requests.js`.
 */
import { get } from "./api.js";
import { data, ui, sinceFor, setScope, pagedList, panelRuleId } from "./state.js";
import { fetchFresh, forgetScope, generation, captureScope, inScope, sendInScope, scopeQuery, repositoryFilter } from "./scoped-requests.js";
import { loadFirstPage } from "./paged-lists.js";
import { loadSession, loadSessionFindings } from "./session-detail.js";
import { loadMergeAssist, forgetMergeAssistRead, MERGE_ASSIST_COUNT_EVERY } from "./merge-assist.js";
import { setTitleListener } from "./session-titles.js";
import { toStoredRules } from "../model/derive.js";
import { historyRuleId } from "../model/history-model.js";
import { failed } from "../format.js";

export { toStoredRules };

const INTERVAL = 5_000;
const MAX_BACKOFF = 30_000;
/** Page sizes: History's selected rule's violations, a rule sheet's, the sessions list. */
const FINDINGS_PAGE = 25;
/** Overview's Recent sessions. */
const RECENT_SESSIONS = 5;
const RULE_FINDINGS_PAGE = 50;
const SESSIONS_PAGE = 50;

let onChange = () => {};
let timer = null;
let backoff = INTERVAL;
let inFlight = null;
let queued = false;

async function loadRepositories() {
  const body = await fetchFresh("/api/repositories", "repositories");
  if (body === undefined) return false;
  data.repositories = body.repositories;
  return true;
}

async function loadContext() {
  const body = await fetchFresh(`/api/repository?${scopeQuery()}`, "context");
  if (body === undefined) return false;
  data.repository = body.repository;
  data.checkout = body.checkout;
  return true;
}

/**
 * The packs added to the repository: pack id → every folder it is attached at, from the rules present
 * through a pack and the pack attachments reported unavailable.
 */
export function attachedPacksOf(entries, unavailable) {
  const packs = new Map();
  const add = (id, folder) => {
    const folders = packs.get(id) ?? [];
    if (!folders.includes(folder)) packs.set(id, [...folders, folder]);
  };
  for (const entry of entries) if (entry.source.kind === "pack") add(entry.source.id, entry.folder);
  for (const reference of unavailable) if (reference.sourceKind === "pack") add(reference.sourceId, reference.folder);
  return packs;
}

async function loadRules() {
  const body = await fetchFresh(`/api/repository/rules?${scopeQuery()}`, "rules");
  if (body === undefined) return false;
  data.rules = toStoredRules(body.rules);
  data.attachedPacks = attachedPacksOf(body.rules, body.unavailable);
  return true;
}

/** The installed packs, and the scoped checkout's own rules (`.rerail/rules`) as the pack `worktree`. */
async function loadPacks() {
  const checkout = ui.scope?.checkout;
  const body = await fetchFresh(checkout === undefined ? "/api/rule-packs" : `/api/rule-packs?${new URLSearchParams({ checkout })}`, "packs");
  if (body === undefined) return false;
  data.packs = body.packs;
  data.packsSkipped = body.skipped;
  return true;
}

/** The daemon's health. A daemon that restarted may run Merge Assist now, so one that said it did not is asked again. */
async function loadHealth() {
  const { body } = await get("/healthz");
  const changed = data.health?.version !== body.version || data.health?.pid !== body.pid;
  if (changed && data.health !== null && data.mergeAssistExperiment === false) data.mergeAssistExperiment = null;
  data.health = body;
  return changed;
}

/** Settings: which coding agents are found and connected. A dry run on the daemon, so only on demand. */
export async function loadAgents() {
  const { body } = await get("/api/coding-agents");
  data.agents = body.agents;
}

/** Settings: the database-wide status line preference; no row means Show. */
export async function loadPresence() {
  const { body } = await get("/api/presentation-preferences");
  data.presence = body.preferences.find(p => p.repository === "" && p.host === "")?.presence ?? "compact";
}

/** Settings: the scoped repository's Follow .gitignore; kept for the page only while it still shows that scope. */
export async function loadFollowGitignore() {
  const scope = captureScope();
  const { body } = await get(`/api/repository?${new URLSearchParams(scope.target)}`);
  if (inScope(scope)) data.followGitignore = body.repository.followGitignore;
}

/**
 * Settings: turn the scoped repository's Follow .gitignore on or off. The page shows it at once and puts it back, with
 * a toast, when the write fails; nothing of the answer lands once the page shows another repository.
 */
export async function saveFollowGitignore(follow, { toast, invalidate }) {
  const scope = captureScope(), was = data.followGitignore;
  data.followGitignore = follow; invalidate();
  try {
    if (!(await sendInScope(scope, "PUT", "/api/repository/follow-gitignore", { followGitignore: follow }))) return;
    toast(follow ? "New sessions skip Git-ignored files." : "New sessions include Git-ignored files.");
  } catch (error) {
    data.followGitignore = was;
    toast(failed("Couldn’t change Follow .gitignore", error));
  }
  invalidate();
}

/**
 * The packs that fit the scoped repository, each with the folder to add it in: W0, Review, and the folder
 * Add pack uses. Reads the repository, so only when the page needs them.
 */
export async function loadRecommendations() {
  await recommendationsFor(captureScope());
}

/** The packs recommended for a captured scope's target; kept for the page only while it still shows that scope. */
export async function recommendationsFor(scope) {
  const { body } = await get(`/api/rule-packs/recommendations?${new URLSearchParams(scope.target)}`);
  if (inScope(scope)) data.recommendations = body.recommendations;
  return body.recommendations;
}

async function loadActivity(since, target) {
  const body = await fetchFresh(`/api/rules/activity?since=${since}${repositoryFilter()}`, target);
  if (body === undefined) return false;
  data[target] = body.rules;
  data[`${target}Sessions`] = body.sessions;
  return true;
}

/** Overview's Recent sessions: the range's newest few, and how many started in it. */
async function loadRecentSessions(since) {
  const body = await fetchFresh(`/api/sessions?since=${since}&limit=${RECENT_SESSIONS}${repositoryFilter()}`, "recentSessions");
  if (body === undefined) return false;
  data.recentSessions = { rows: body.sessions, total: body.total };
  return true;
}

/**
 * What a page can ask a poll for, by name; each loads with the range's `since`. A view declares the names it shows
 * (`needs` in its module) and the poll loads each name once, whoever else asked for it.
 * - `rules`: the repository's rules (`data.rules`, `attachedPacks`).
 * - `packs`: every rule pack installed on this machine, and the scoped checkout's own rules (`data.packs`).
 * - `activity`: each rule's activity in the range and since install (`data.activity`, `activityAll`, their sessions).
 * - `recentSessions`: the range's five newest sessions and how many started in it (`data.recentSessions`).
 * - `findings`: History's shown rule's violations in the range (`data.findings`, owner = rule id); nothing without one.
 * - `ruleFindings`: the open rule sheet's violations in the range (`data.ruleFindings`); nothing without one.
 * - `sessions`: the first page of the sessions list (`data.sessions`).
 * - `selectedSession`: the selected session's head, new transcript events and violations; nothing without one.
 * - `mergeAssist`: Merge Assist for the scoped checkout (`data.mergeAssist`), every poll.
 * - `mergeAssistCount`: the same, read at most every 30 s: the sidebar's count of open conflicts. Not in a scope's
 *   first poll, which the page waits on to draw, so the count never holds the page up.
 */
const NEEDS = {
  rules: () => loadRules(),
  packs: () => loadPacks(),
  activity: since => [loadActivity(since, "activity"), since === 0 ? false : loadActivity(0, "activityAll")],
  recentSessions: since => loadRecentSessions(since),
  findings: since => {
    const ruleId = historyRuleId();
    return ruleId === null ? [] : loadFirstPage("findings", "violations",
      `/api/violations?since=${since}&ruleId=${encodeURIComponent(ruleId)}&limit=${FINDINGS_PAGE}${repositoryFilter()}`, ruleId);
  },
  ruleFindings: since => {
    const ruleId = panelRuleId();
    return ruleId === null ? [] : loadFirstPage("ruleFindings", "violations",
      `/api/violations?since=${since}&ruleId=${encodeURIComponent(ruleId)}&limit=${RULE_FINDINGS_PAGE}${repositoryFilter()}`, ruleId);
  },
  sessions: () => loadFirstPage("sessions", "sessions", `/api/sessions?limit=${SESSIONS_PAGE}${repositoryFilter()}`),
  selectedSession: () => (ui.selectedSession ? [loadSession(ui.selectedSession), loadSessionFindings(ui.selectedSession)] : []),
  mergeAssist: () => loadMergeAssist(),
  mergeAssistCount: () => (data.loaded ? loadMergeAssist(MERGE_ASSIST_COUNT_EVERY) : false),
};

/** The names of what the page needs now, beyond the shared reads; set once by the page (`setNeeds`). */
let pageNeeds = () => [];

/** Name what each poll loads for a repository in scope: `needs()` returns names from `NEEDS`, in any order, repeats allowed. */
export function setNeeds(needs) {
  pageNeeds = needs;
}

function neededJobs(since) {
  const jobs = [];
  for (const name of new Set(pageNeeds())) {
    const load = NEEDS[name];
    if (load === undefined) throw new Error(`No poll need is called "${name}"`);
    jobs.push(...[load(since)].flat());
  }
  return jobs;
}

/** The scoped repository's context (first, when its key is not known yet), then what the page needs. */
async function scopedJobs(since) {
  const context = data.repository === null ? [await loadContext()] : [loadContext()];
  return [...context, ...neededJobs(since)];
}

/** A poll answered: what it read is the page's now, and the daemon is reachable. */
function pollAnswered(since, results) {
  if (since === 0) { data.activityAll = data.activity; data.activityAllSessions = data.activitySessions; }
  data.updatedAt = Date.now();
  data.disconnected = false;
  data.accessRequired = false;
  data.missing = false;
  data.loaded = true;
  backoff = INTERVAL;
  onChange(results.some(Boolean));
}

/**
 * A poll failed with `error`. A repository or checkout the daemon does not know is an answer, not a lost connection:
 * the context or rules read answering 404, not some other read (a session that went away is handled where it is
 * read). Neither is a 401: the daemon is up, and this browser has not opened its link.
 */
function pollFailed(error) {
  data.accessRequired = error.name === "AccessRequired";
  data.missing = !data.accessRequired && /^\/api\/repository(\/rules)?\?.* answered 404$/u.test(error.message);
  data.disconnected = !data.missing && !data.accessRequired;
  data.loaded = data.missing;
  backoff = data.missing ? INTERVAL : Math.min(MAX_BACKOFF, backoff * 2);
  if (!data.missing) console.warn("poll failed", error);
  onChange(data.missing);
}

async function poll() {
  const since = sinceFor();
  const started = generation;
  try {
    const shared = [loadRepositories(), loadHealth()];
    const results = await Promise.all(ui.scope === null ? shared : [...shared, ...await scopedJobs(since)]);
    if (started !== generation) return;
    pollAnswered(since, results);
  } catch (error) {
    if (started !== generation) return;
    pollFailed(error);
  }
}

/**
 * One poll: everything the current view needs, in parallel. A call during a poll joins it and
 * asks for one more afterwards, since what to fetch (the range) may have changed meanwhile.
 */
export async function refresh() {
  if (inFlight) { queued = true; return inFlight; }
  inFlight = poll();
  try { await inFlight; } finally { inFlight = null; }
  if (queued) { queued = false; await refresh(); }
}

/** The tab is in the background: nothing is polled until it shows again. */
const hidden = () => typeof document !== "undefined" && document.hidden;

function schedule() {
  clearTimeout(timer);
  if (hidden()) return;
  timer = setTimeout(async () => { await refresh(); schedule(); }, backoff);
}

export function startPolling(listener) {
  onChange = listener;
  setTitleListener(listener);
  refresh().then(schedule);
  document.addEventListener("visibilitychange", () => { if (hidden()) clearTimeout(timer); else refreshNow(); });
}

/** A user action that changes what to fetch (view, range, selected session) wants data now. */
export async function refreshNow() {
  await refresh();
  schedule();
}

/**
 * Show another repository (`scope` from `readScope`, or `null` for the page that asks for one): drop everything fetched for the
 * previous one, and poll now. A response still in flight for the previous scope is discarded.
 */
export function changeScope(scope) {
  forgetScope();
  setScope(scope);
  ui.panel = null;
  ui.backToPack = null;
  ui.removeAsk = null;
  ui.removedPacks.clear();
  ui.selectedSession = null;
  ui.sessRule = null;
  ui.mergeDismissing.clear();
  ui.mergeOpen.clear();
  ui.mergeSettings = null;
  ui.mergeSaving = false;
  ui.mergeSwitching = false;
  ui.mergeScanning = false;
  Object.assign(data, {
    repository: null, checkout: null, rules: new Map(), attachedPacks: new Map(), activity: {}, activityAll: {},
    activitySessions: null, activityAllSessions: null, findings: pagedList(), ruleFindings: pagedList(), sessions: pagedList(),
    sessionFindings: pagedList(), recentSessions: null, recommendations: null, followGitignore: null, loaded: false, missing: false,
    mergeAssist: null, mergeAssistError: null,
  });
  forgetMergeAssistRead();
  data.sessionDetail.clear();
  refreshNow();
}
