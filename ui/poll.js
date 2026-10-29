/**
 * Fetch orchestration: one conditional GET per resource every 5 s (`If-None-Match`; 304 means
 * nothing changed), backing off to 30 s while the daemon is unreachable, and paused while the tab is hidden.
 * Data lands in the store; the caller re-renders and reconciles in place. Every poll reads the repository
 * list and the daemon's health, and with a repository in scope its context; beyond that only the needs the
 * page names (`setNeeds`: the open view's, the sidebar's and an open sheet's, from `views/index.js`). Long lists
 * are paged with a cursor: a poll refreshes their first page and keeps the further pages the person loaded.
 */
import { get, send } from "./api.js";
import { data, ui, sinceFor, setScope, pagedList, panelRuleId } from "./state.js";
import { historyRuleId } from "./history-model.js";

const INTERVAL = 5_000;
const MAX_BACKOFF = 30_000;
/** Page sizes: History's selected rule's violations, a rule sheet's, the sessions list, the selected session's violations, its transcript. */
const FINDINGS_PAGE = 25;
/** Overview's Recent sessions. */
const RECENT_SESSIONS = 5;
const RULE_FINDINGS_PAGE = 50;
const SESSIONS_PAGE = 50;
const SESSION_FINDINGS_PAGE = 200;
const EVENTS_PAGE = 500;

/**
 * ETag per data slot, not per URL: range-scoped resources fetch a different URL for every range,
 * and the tag must describe what `data.*` currently holds so that returning to a range whose body
 * the daemon already served once still answers 200 instead of a 304 that leaves the other range's
 * data in place.
 */
const etags = new Map();
let onChange = () => {};
let timer = null;
let backoff = INTERVAL;
let inFlight = null;
let queued = false;
/** Bumped when the scope changes, so a response for the previous repository lands nowhere. */
let generation = 0;

/**
 * Conditional GET; resolves `undefined` on 304, after a scope change, or once `wanted()` says the answer no longer
 * belongs where it would land (its ETag is then not kept either); the body on 200. `slot` names the ETag's data slot.
 */
async function fetchFresh(path, slot = path, wanted = () => true) {
  const started = generation;
  const { status, etag, body } = await get(path, { etag: etags.get(slot) });
  if (started !== generation || status === 304 || !wanted()) return undefined;
  if (etag !== undefined) etags.set(slot, etag);
  return body;
}

/**
 * Fetch slot `slot` in full on the next poll, not conditionally: after a write failed, what the page shows was
 * changed by hand and may differ from what the daemon holds although the daemon's body did not change.
 */
export const refetch = slot => { etags.delete(slot); };

/** `repository=…&checkout=…` from the page's scope: the target every repository read and write names. */
function scopeTarget() {
  const target = {};
  const repository = data.repository?.repositoryKey ?? ui.scope?.repository;
  if (repository !== undefined) target.repository = repository;
  if (ui.scope?.checkout !== undefined) target.checkout = ui.scope.checkout;
  return target;
}

/**
 * The scope a write belongs to, fixed at the click: its `target` and a `token` that goes stale once the page
 * shows another repository. A write sends to `target` and touches the page's state only while `inScope`.
 */
export const captureScope = () => ({ target: scopeTarget(), token: generation });

/** Whether the page still shows the scope `captureScope` fixed. */
export const inScope = scope => scope.token === generation;

/**
 * One write to the captured scope's target. Nothing is sent once the page shows another repository. Resolves
 * `true` when sent and the page still shows the scope, `false` when it does not (so nothing of the answer is the
 * page's); rejects with the write's error only while the page still shows the scope, so a rollback lands there.
 */
export async function sendInScope(scope, method, path, body) {
  if (!inScope(scope)) return false;
  try {
    await send(method, path, { ...scope.target, ...body });
  } catch (error) {
    if (inScope(scope)) throw error;
    return false;
  }
  return inScope(scope);
}

/**
 * The last attachment write queued on this page: the scope `token` it was clicked in, whether it is still
 * `pending`, whether it was sent and answered OK (`clear`), and `settled` once those are known.
 */
let lastAttachmentWrite = { token: generation, pending: false, clear: true, settled: Promise.resolve() };

/**
 * `sendInScope` for attachment writes, in click order. The place is taken at the call, while `body` (the body, or a
 * promise of it, `null` to send nothing) may still be loading. A write called while an earlier one in the same
 * scope is still pending is sent once that one is answered OK, and dropped (resolves `false`, nothing sent) when it
 * failed or was dropped. Each was clicked against the state the earlier ones would make: a pack added behind a
 * set-up that failed would set the repository up with that pack alone. Rejects when its own body fails to load, as
 * when its send fails, and like a send only while the page still shows the scope: a failure after a switch resolves
 * `false`, so no rollback lands on the other repository's page. A scope switch starts a new queue.
 */
export function queueInScope(scope, method, path, body) {
  const before = lastAttachmentWrite;
  const waits = before.pending && before.token === scope.token;
  const ready = Promise.resolve(body);
  // Read only once the writes before it went out OK; a dropped write's failing body is nobody's error.
  ready.catch(() => {});
  const sending = (async () => {
    if (waits) {
      await before.settled;
      if (!before.clear) return false;
    }
    let loaded;
    try {
      loaded = await ready;
    } catch (error) {
      if (inScope(scope)) throw error;
      return false;
    }
    return loaded === null ? false : sendInScope(scope, method, path, loaded);
  })();
  const write = { token: scope.token, pending: true, clear: false, settled: null };
  write.settled = sending.then(
    sent => { write.pending = false; write.clear = sent; },
    () => { write.pending = false; },
  );
  lastAttachmentWrite = write;
  return sending;
}

const scopeQuery = () => new URLSearchParams(scopeTarget()).toString();

/** `&repository=<key>` for the machine-wide reads (activity, violations, sessions), once the key is known. */
function repositoryFilter() {
  const key = data.repository?.repositoryKey ?? ui.scope?.repository;
  return key === undefined ? "" : `&repository=${encodeURIComponent(key)}`;
}

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
 * One row per rule id, in the renderers' shape: on (`enabled`, the switch) only where it is on in every folder it is
 * added at, the first folder's mode, suppressions and options on the rule, and each folder's own on, mode,
 * suppressions and what decided them (`folderStates`; `mixed` when on or mode differ). `configError` is why the rule's
 * options don't apply, from any folder: the rule then runs nowhere, whatever its switch says. A change from the row is
 * written at every folder.
 */
export function toStoredRules(entries) {
  const rules = new Map();
  for (const entry of entries) {
    const id = entry.rule.id;
    const known = rules.get(id);
    if (known !== undefined) {
      if (!known.folders.includes(entry.folder)) {
        known.folders.push(entry.folder);
        known.folderStates.push(folderState(entry));
        known.enabled = known.enabled && entry.on;
        known.configError ??= entry.configError ?? null;
        known.mixed = known.folderStates.some(state => state.on !== entry.on || state.mode !== entry.mode) || known.mixed;
        known.addedAt = earliest(known.addedAt, entry.addedAt ?? null);
        known.updatedAt = latest(known.updatedAt, entry.updatedAt ?? null);
      }
      continue;
    }
    rules.set(id, {
      kind: entry.kind,
      rule: { ...entry.rule, mode: entry.mode },
      enabled: entry.on,
      origin: entry.source.kind === "library_rule" ? "user" : "rerail",
      source: entry.source,
      folders: [entry.folder],
      folderStates: [folderState(entry)],
      mixed: false,
      onSource: entry.onSource,
      modeSource: entry.modeSource,
      suppressions: entry.suppressions,
      suppressionsSource: entry.suppressionsSource,
      config: entry.config,
      configSource: entry.configSource,
      configError: entry.configError ?? null,
      revision: entry.revision,
      addedAt: entry.addedAt ?? null,
      updatedAt: entry.updatedAt ?? null,
    });
  }
  return rules;
}

/** When a rule present in several folders was first added: the earliest folder's; `null` where none says. */
const earliest = (a, b) => (a === null ? b : b === null ? a : Math.min(a, b));
/** When its settings last changed: the latest folder's. */
const latest = (a, b) => (a === null ? b : b === null ? a : Math.max(a, b));

/** One folder's own on, mode and suppressions of a rule, what decided its suppressions, and what added it there. */
const folderState = entry => ({
  folder: entry.folder, on: entry.on, mode: entry.mode, suppressions: entry.suppressions, suppressionsSource: entry.suppressionsSource,
  source: entry.source,
});

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

async function loadPacks() {
  const body = await fetchFresh("/api/rule-packs", "packs");
  if (body === undefined) return false;
  data.packs = body.packs;
  return true;
}

async function loadHealth() {
  const { body } = await get("/healthz");
  const changed = data.health?.version !== body.version || data.health?.pid !== body.pid;
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
    toast(follow ? "Files Git ignores are not checked in sessions started from now on." : "Files Git ignores are checked in sessions started from now on.");
  } catch (error) {
    data.followGitignore = was;
    toast(`Could not save: ${error.message}`);
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

/** How a row of each paged resource is told apart from the others. */
const ROW_ID = { violations: r => `${r.auditId}:${r.id}`, sessions: r => r.sessionId };

/**
 * The first page of the paged list `data[name]` at `key` (its URL without a cursor; `field` names the rows in the
 * answer), conditionally. Another key starts the list over at once, dropping the previous key's pages. Under the
 * same key, pages loaded beyond the first stay, deduplicated, with the cursor past the last of them, as long as the
 * fresh page reaches them: when more rows arrived than a page holds, the rows between the fresh page and the kept
 * ones could never be loaded, so the list starts over from the fresh page and its own cursor.
 */
async function loadFirstPage(name, field, key, owner = null) {
  if (data[name].key !== key) {
    etags.delete(name);
    data[name] = pagedList(key, owner);
  }
  const body = await fetchFresh(key, name, () => data[name].key === key);
  if (body === undefined) return false;
  const current = data[name], rows = body[field];
  const id = ROW_ID[field];
  const kept = new Set(current.rows.map(id));
  // The fresh page is the whole list, or it does not reach the kept pages: it replaces them.
  const complete = body.nextCursor === null;
  const reaches = rows.length > 0 && kept.has(id(rows.at(-1)));
  if (current.pages <= 1 || complete || !reaches) {
    data[name] = { ...current, rows, total: body.total, next: body.nextCursor, pages: 1 };
    return true;
  }
  const fresh = new Set(rows.map(id));
  data[name] = { ...current, rows: rows.concat(current.rows.filter(r => !fresh.has(id(r)))), total: body.total };
  return true;
}

/**
 * The page after `data[name]`'s cursor, appended without duplicates. `false` when there is none, or when the list
 * moved on meanwhile (another key or scope, or this page already appended by an earlier click).
 */
async function loadNextPage(name, field) {
  const { key, next } = data[name];
  if (key === null || next === null) return false;
  const started = generation;
  const { body } = await get(`${key}&cursor=${encodeURIComponent(next)}`);
  const current = data[name];
  if (started !== generation || current.key !== key || current.next !== next) return false;
  const id = ROW_ID[field], have = new Set(current.rows.map(id));
  data[name] = {
    ...current, rows: current.rows.concat(body[field].filter(r => !have.has(id(r)))),
    total: body.total, next: body.nextCursor, pages: current.pages + 1,
  };
  return true;
}

/** History: the next page of the shown rule's violations. */
export const loadMoreFindings = () => loadNextPage("findings", "violations");

/** Sessions view: the next page of the list. */
export const loadMoreSessions = () => loadNextPage("sessions", "sessions");

/**
 * The selected session's violations, all time, newest first; selecting another session drops the previous one's.
 * Their total goes into the session's list row too, so its count, the With violations filter and its day agree
 * however old the page the row came with.
 */
export async function loadSessionFindings(id) {
  const changed = await loadFirstPage("sessionFindings", "violations",
    `/api/violations?since=0&sessionId=${encodeURIComponent(id)}&limit=${SESSION_FINDINGS_PAGE}${repositoryFilter()}`, id);
  const slot = data.sessionFindings;
  if (slot.owner === id && slot.pages > 0) patchSessionRow({ sessionId: id, violationCount: slot.total });
  return changed;
}

/** One page of a session's transcript, `query` saying where (`after=…` or `before=…`). */
async function eventsPage(id, query) {
  const { body } = await get(`/api/sessions/${encodeURIComponent(id)}/events?${query}&limit=${EVENTS_PAGE}`);
  return body;
}

/** Runs `load` as the session's one transcript load; a call while one runs joins it. */
async function sessionLoad(detail, load) {
  if (detail.loading) return detail.loading;
  detail.loading = load();
  try { return await detail.loading; } finally { detail.loading = null; }
}

/** Writes a session's fresh fields over its row in the loaded list, so a row past the first page stays current too. */
function patchSessionRow(fields) {
  const rows = data.sessions.rows;
  if (!rows.some(s => s.sessionId === fields.sessionId)) return;
  data.sessions = { ...data.sessions, rows: rows.map(s => (s.sessionId === fields.sessionId ? { ...s, ...fields } : s)) };
}

/** Forgets a session the daemon no longer has; the view then falls back to another, as for a session never selected. */
function dropSession(id) {
  data.sessionDetail.delete(id);
  data.sessions = { ...data.sessions, rows: data.sessions.rows.filter(s => s.sessionId !== id) };
  if (data.sessionFindings.owner === id) data.sessionFindings = pagedList();
  if (ui.selectedSession === id) ui.selectedSession = null;
}

/**
 * One session's head and the newest page of its transcript; later calls refresh the head (it is one row, and the
 * list may hold an old copy of it) and append the events recorded since. The head is fetched before any events: a
 * session missing on its first load fails with its 404 rather than looking like an empty transcript, while one that
 * disappears later (a reset) is dropped. Older events come with Load earlier (`loadEarlier`); `earlier` says some
 * may be left. `gaps` is what the whole transcript says it missed, from the daemon.
 */
export async function loadSession(id) {
  let detail = data.sessionDetail.get(id);
  if (detail === undefined) {
    detail = { head: null, gaps: null, events: [], nextAfter: 0, loaded: false, earlier: false, loading: null };
    data.sessionDetail.set(id, detail);
  }
  return sessionLoad(detail, async () => {
    let body;
    try {
      ({ body } = await get(`/api/sessions/${encodeURIComponent(id)}`));
    } catch (error) {
      if (!detail.loaded || !/answered 404$/u.test(error.message)) throw error;
      dropSession(id);
      return true;
    }
    const was = detail.head, wasGaps = detail.gaps;
    detail.head = body.session;
    detail.gaps = body.gaps;
    patchSessionRow(body.session);
    const changed = was?.eventCount !== body.session.eventCount || was?.endedAt !== body.session.endedAt
      || JSON.stringify(wasGaps) !== JSON.stringify(body.gaps);
    const page = await eventsPage(id, detail.loaded ? `after=${detail.nextAfter}` : `before=${Number.MAX_SAFE_INTEGER}`);
    if (!detail.loaded) detail.earlier = page.events.length === EVENTS_PAGE;
    detail.loaded = true;
    detail.nextAfter = page.nextAfter;
    if (page.events.length === 0) return changed;
    detail.events = detail.events.concat(page.events);
    return true;
  });
}

/**
 * Load earlier: reach one page further back in the selected session. The transcript and the session's violations
 * page back together (`loadedFrom` in `sessions-model.js`): whichever reaches back less far loads its next page.
 */
export async function loadEarlier(id) {
  const detail = data.sessionDetail.get(id);
  if (detail?.loading) await detail.loading.catch(() => {});
  if (detail === undefined || !detail.loaded) return false;
  const slot = data.sessionFindings;
  const oldestViolation = slot.rows.at(-1)?.at ?? Infinity;
  const oldestEvent = detail.earlier ? detail.events[0]?.observedAt ?? -Infinity : -Infinity;
  if (slot.owner === id && slot.next !== null && oldestViolation >= oldestEvent) return loadNextPage("sessionFindings", "violations");
  if (!detail.earlier) return false;
  return sessionLoad(detail, async () => {
    const page = await eventsPage(id, `before=${detail.events[0]?.sequence ?? Number.MAX_SAFE_INTEGER}`);
    detail.earlier = page.events.length === EVENTS_PAGE;
    detail.events = page.events.concat(detail.events);
    return page.events.length > 0;
  });
}

const WEAK = /^(continue|yes|no|ok|okay|go|try again|thanks|thank you|do it|next|y|n)\.?$/i;
const TITLE_MAX = 96;
/** First user prompt with three or more words that is not a bare acknowledgement; its first line, trimmed to a title. */
export function titleFromPrompts(prompts) {
  const t = prompts.find(u => u.split(/\s+/).length >= 3 && !WEAK.test(u.trim())) || prompts[0];
  if (!t) return null;
  const line = t.trim().split("\n")[0].replace(/\s+/g, " ").trim();
  return line.length > TITLE_MAX ? `${line.slice(0, TITLE_MAX - 1).trimEnd()}…` : line;
}

const titleQueue = [];
let titleWorkers = 0;
const TITLE_WORKERS = 4;
/** sessionId → `eventCount` the current or last lookup covered; a retry waits for new events. */
const titleTried = new Map();

function titleFromEvents(events) {
  return titleFromPrompts(events.filter(e => e.kind === "user-prompt").map(e => String(e.payload?.text ?? "")).filter(Boolean));
}

/**
 * Titles come from the first user prompts; sessions_list carries none, so they load lazily per visible
 * row. Cached transcripts resolve synchronously, so the caller's `titleOf` sees the result in the same
 * render. A session without a usable prompt stays "Untitled" until it records more events; it is never
 * re-fetched (or re-rendered) just because it rendered again.
 */
export function ensureTitle(session) {
  const id = session.sessionId;
  if (typeof data.titles.get(id) === "string") return;
  const tried = titleTried.get(id);
  if (tried !== undefined && (session.endedAt !== null || session.eventCount <= tried)) return;
  titleTried.set(id, session.eventCount);
  // Only a transcript loaded from its start holds the first prompts; a newer page would name a later one.
  const detail = data.sessionDetail.get(id);
  if (detail?.loaded && !detail.earlier && detail.events.length) { data.titles.set(id, titleFromEvents(detail.events)); return; }
  if (!titleQueue.includes(id)) titleQueue.push(id);
  pumpTitles();
}

function pumpTitles() {
  while (titleWorkers < TITLE_WORKERS && titleQueue.length) {
    const id = titleQueue.shift();
    titleWorkers += 1;
    (async () => {
      try {
        const { body } = await get(`/api/sessions/${encodeURIComponent(id)}/events?after=0&limit=60`);
        const title = titleFromEvents(body.events);
        if (title !== null || data.titles.get(id) === undefined) data.titles.set(id, title);
        onChange();
      } catch { titleTried.delete(id); /* the row keeps "Untitled session" and retries on a later render */ }
      finally { titleWorkers -= 1; pumpTitles(); }
    })();
  }
}

/**
 * What a page can ask a poll for, by name; each loads with the range's `since`. A view declares the names it shows
 * (`needs` in its module) and the poll loads each name once, whoever else asked for it.
 * - `rules`: the repository's rules (`data.rules`, `attachedPacks`).
 * - `packs`: every rule pack that comes with Rerail (`data.packs`).
 * - `activity`: each rule's activity in the range and since install (`data.activity`, `activityAll`, their sessions).
 * - `recentSessions`: the range's five newest sessions and how many started in it (`data.recentSessions`).
 * - `findings`: History's shown rule's violations in the range (`data.findings`, owner = rule id); nothing without one.
 * - `ruleFindings`: the open rule sheet's violations in the range (`data.ruleFindings`); nothing without one.
 * - `sessions`: the first page of the sessions list (`data.sessions`).
 * - `selectedSession`: the selected session's head, new transcript events and violations; nothing without one.
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

async function poll() {
  const since = sinceFor();
  const started = generation;
  try {
    const shared = [loadRepositories(), loadHealth()];
    const results = await Promise.all(ui.scope === null ? shared : [...shared, ...await scopedJobs(since)]);
    if (started !== generation) return;
    if (since === 0) { data.activityAll = data.activity; data.activityAllSessions = data.activitySessions; }
    data.updatedAt = Date.now();
    data.disconnected = false;
    data.accessRequired = false;
    data.missing = false;
    data.loaded = true;
    backoff = INTERVAL;
    onChange(results.some(Boolean));
  } catch (error) {
    if (started !== generation) return;
    // A repository or checkout the daemon does not know is an answer, not a lost connection: the context or rules read
    // answering 404, not some other read (a session that went away is handled where it is read). Neither is a 401: the
    // daemon is up, and this browser has not opened its link.
    data.accessRequired = error.name === "AccessRequired";
    data.missing = !data.accessRequired && /^\/api\/repository(\/rules)?\?.* answered 404$/u.test(error.message);
    data.disconnected = !data.missing && !data.accessRequired;
    data.loaded = data.missing;
    backoff = data.missing ? INTERVAL : Math.min(MAX_BACKOFF, backoff * 2);
    if (!data.missing) console.warn("poll failed", error);
    onChange(data.missing);
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
  generation += 1;
  etags.clear();
  setScope(scope);
  ui.panel = null;
  ui.backToPack = null;
  ui.removeAsk = null;
  ui.removedPacks.clear();
  ui.selectedSession = null;
  ui.sessRule = null;
  Object.assign(data, {
    repository: null, checkout: null, rules: new Map(), attachedPacks: new Map(), activity: {}, activityAll: {},
    activitySessions: null, activityAllSessions: null, findings: pagedList(), ruleFindings: pagedList(), sessions: pagedList(),
    sessionFindings: pagedList(), recentSessions: null, recommendations: null, followGitignore: null, loaded: false, missing: false,
  });
  data.sessionDetail.clear();
  refreshNow();
}
