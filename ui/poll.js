/**
 * Fetch orchestration: one conditional GET per resource every 5 s (`If-None-Match`; 304 means
 * nothing changed), backing off to 30 s while the daemon is unreachable. Data lands in the
 * store; the caller re-renders and reconciles in place. With a repository in scope the page
 * polls its context, its rules (and the packs attached to it), the rule packs, its activity,
 * violations and sessions; with or without one, the repository list and the daemon's health.
 */
import { get, send } from "./api.js";
import { data, ui, sinceFor, setScope, activityView } from "./state.js";

const INTERVAL = 5_000;
const MAX_BACKOFF = 30_000;
const FINDINGS_PAGE = 200;
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

/** Conditional GET; resolves `undefined` on 304 or after a scope change, the body on 200. `slot` names the ETag's data slot. */
async function fetchFresh(path, slot = path) {
  const started = generation;
  const { status, etag, body } = await get(path, { etag: etags.get(slot) });
  if (started !== generation || status === 304) return undefined;
  if (etag !== undefined) etags.set(slot, etag);
  return body;
}

/** `repository=…&checkout=…` from the page's scope: the target every repository read and write names. */
export function scopeTarget() {
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
 * One row per rule id, in the renderers' shape: on (`enabled`) only where it is on in every folder it is added at,
 * the first folder's mode, suppressions and options on the rule, and each folder's own on, mode and suppressions
 * (`folderStates`; `mixed` when on or mode differ). A change from the row is written at every folder.
 */
export function toStoredRules(entries) {
  const rules = new Map();
  for (const entry of entries) {
    const id = entry.rule.id;
    const known = rules.get(id);
    if (known !== undefined) {
      if (!known.folders.includes(entry.folder)) {
        known.folders.push(entry.folder);
        known.folderStates.push({ folder: entry.folder, on: entry.on, mode: entry.mode, suppressions: entry.suppressions });
        known.enabled = known.enabled && entry.on;
        known.mixed = known.folderStates.some(state => state.on !== entry.on || state.mode !== entry.mode) || known.mixed;
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
      folderStates: [{ folder: entry.folder, on: entry.on, mode: entry.mode, suppressions: entry.suppressions }],
      mixed: false,
      onSource: entry.onSource,
      modeSource: entry.modeSource,
      suppressions: entry.suppressions,
      suppressionsSource: entry.suppressionsSource,
      config: entry.config,
      configSource: entry.configSource,
      revision: entry.revision,
    });
  }
  return rules;
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
  data.unavailable = body.unavailable;
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
  return true;
}

async function loadFindings(since) {
  const body = await fetchFresh(`/api/violations?since=${since}&limit=${FINDINGS_PAGE}&offset=0${repositoryFilter()}`, "findings");
  if (body === undefined) { data.findingsSince = since; return false; }
  // Keep pages the user asked for beyond the first, but only if they still belong to this range.
  const tail = data.findings.length > FINDINGS_PAGE && data.findingsSince === since ? data.findings.slice(FINDINGS_PAGE) : [];
  data.findings = body.violations.concat(tail.filter(f => !body.violations.some(g => g.auditId === f.auditId && g.id === f.id)));
  data.findingsTotal = body.total;
  data.findingsSince = since;
  return true;
}

/** "Show 50 more" past the fetched pages appends the next page of the same range. */
export async function loadMoreFindings() {
  const since = sinceFor();
  const offset = data.findings.length;
  if (offset >= data.findingsTotal) return false;
  const started = generation;
  const { body } = await get(`/api/violations?since=${since}&limit=${FINDINGS_PAGE}&offset=${offset}${repositoryFilter()}`);
  if (data.findingsSince !== since || started !== generation) return false;
  data.findings = data.findings.concat(body.violations);
  data.findingsTotal = body.total;
  return body.violations.length > 0;
}

/**
 * Activity's "Show more": 50 more rows of what the filters let through, fetching further pages until
 * that many matches are loaded or the range runs out (a filter's matches may all lie further back).
 */
export async function showMoreActivity() {
  ui.recentShown += 50;
  while (activityView().list.length < ui.recentShown && await loadMoreFindings()) { /* next page */ }
}

async function loadSessions() {
  const body = await fetchFresh(`/api/sessions?${repositoryFilter().slice(1)}`, "sessions");
  if (body === undefined) return false;
  data.sessions = body.sessions.slice().sort((a, b) => b.startedAt - a.startedAt || a.sessionId.localeCompare(b.sessionId));
  return true;
}

/**
 * Full transcript for one session, paged by sequence; later calls append only new events. The
 * session head is fetched once, before any events: a missing session must fail with its 404
 * rather than look like an empty transcript.
 */
export async function loadSession(id) {
  let detail = data.sessionDetail.get(id);
  if (detail === undefined) {
    detail = { events: [], nextAfter: 0, loaded: false, loading: null };
    data.sessionDetail.set(id, detail);
  }
  if (detail.loading) return detail.loading;
  detail.loading = (async () => {
    let changed = !detail.loaded;
    if (!detail.loaded) await get(`/api/sessions/${encodeURIComponent(id)}`);
    for (;;) {
      const { body } = await get(`/api/sessions/${encodeURIComponent(id)}/events?after=${detail.nextAfter}&limit=${EVENTS_PAGE}`);
      if (body.events.length === 0) break;
      detail.events.push(...body.events);
      detail.nextAfter = body.nextAfter;
      changed = true;
      if (body.events.length < EVENTS_PAGE) break;
    }
    detail.loaded = true;
    return changed;
  })();
  try { return await detail.loading; } finally { detail.loading = null; }
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
  const cached = data.sessionDetail.get(id)?.events;
  if (cached?.length) { data.titles.set(id, titleFromEvents(cached)); return; }
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

/** Everything the scoped repository's views need; its context first when the key is not known yet. */
async function scopedJobs(since) {
  const context = data.repository === null ? [await loadContext()] : [loadContext()];
  const jobs = [...context, loadRules(), loadPacks(), loadActivity(since, "activity"), loadFindings(since), loadSessions()];
  jobs.push(since === 0 ? Promise.resolve(false) : loadActivity(0, "activityAll"));
  if (ui.selectedSession && ui.view === "sessions") jobs.push(loadSession(ui.selectedSession));
  return jobs;
}

async function poll() {
  const since = sinceFor();
  const started = generation;
  try {
    const shared = [loadRepositories(), loadHealth()];
    const results = await Promise.all(ui.scope === null ? shared : [...shared, ...await scopedJobs(since)]);
    if (started !== generation) return;
    if (since === 0) data.activityAll = data.activity;
    data.updatedAt = Date.now();
    data.disconnected = false;
    data.missing = false;
    data.loaded = true;
    backoff = INTERVAL;
    onChange(results.some(Boolean));
  } catch (error) {
    if (started !== generation) return;
    // A repository or checkout the daemon does not know is an answer, not a lost connection.
    data.missing = /answered 404$/u.test(error.message);
    data.disconnected = !data.missing;
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

function schedule() {
  clearTimeout(timer);
  timer = setTimeout(async () => { await refresh(); schedule(); }, backoff);
}

export function startPolling(listener) {
  onChange = listener;
  refresh().then(schedule);
  document.addEventListener("visibilitychange", () => { if (!document.hidden) { refresh(); schedule(); } });
}

/** A user action that changes what to fetch (range, selected session) wants data now. */
export async function refreshNow() {
  await refresh();
  schedule();
}

/**
 * Show another repository (`scope` from `readScope`, or `null` for the picker): drop everything fetched for the
 * previous one, and poll now. A response still in flight for the previous scope is discarded.
 */
export function changeScope(scope) {
  generation += 1;
  etags.clear();
  setScope(scope);
  ui.panel = null;
  ui.backToPack = null;
  ui.removeAsk = null;
  ui.selectedSession = null;
  Object.assign(data, {
    repository: null, checkout: null, unavailable: [], rules: new Map(), attachedPacks: new Map(), activity: {}, activityAll: {},
    findings: [], findingsTotal: 0, sessions: [], recommendations: null, loaded: false, missing: false,
  });
  data.sessionDetail.clear();
  refreshNow();
}
