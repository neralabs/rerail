/**
 * Fetch orchestration: one conditional GET per resource every 5 s (`If-None-Match`; 304 means
 * nothing changed), backing off to 30 s while the daemon is unreachable. Data lands in the
 * store; the caller re-renders and reconciles in place.
 */
import { get } from "./api.js";
import { data, ui, sinceFor } from "./state.js";

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

/** Conditional GET; resolves `undefined` on 304, the body on 200. `slot` names the ETag's data slot. */
async function fetchFresh(path, slot = path) {
  const { status, etag, body } = await get(path, { etag: etags.get(slot) });
  if (status === 304) return undefined;
  if (etag !== undefined) etags.set(slot, etag);
  return body;
}

async function loadRules() {
  const body = await fetchFresh("/api/rules");
  if (body === undefined) return false;
  const next = new Map();
  for (const stored of body.rules) next.set(stored.rule.id, stored);
  data.rules = next;
  return true;
}

async function loadActivity(since, target) {
  const body = await fetchFresh(`/api/rules/activity?since=${since}`, target);
  if (body === undefined) return false;
  data[target] = body.rules;
  return true;
}

async function loadFindings(since) {
  const body = await fetchFresh(`/api/findings?since=${since}&limit=${FINDINGS_PAGE}&offset=0`, "findings");
  if (body === undefined) { data.findingsSince = since; return false; }
  // Keep pages the user asked for beyond the first, but only if they still belong to this range.
  const tail = data.findings.length > FINDINGS_PAGE && data.findingsSince === since ? data.findings.slice(FINDINGS_PAGE) : [];
  data.findings = body.findings.concat(tail.filter(f => !body.findings.some(g => g.runId === f.runId && g.id === f.id)));
  data.findingsTotal = body.total;
  data.findingsSince = since;
  return true;
}

/** "Show 50 more" past the fetched pages appends the next page of the same range. */
export async function loadMoreFindings() {
  const since = sinceFor();
  const offset = data.findings.length;
  if (offset >= data.findingsTotal) return false;
  const { body } = await get(`/api/findings?since=${since}&limit=${FINDINGS_PAGE}&offset=${offset}`);
  if (data.findingsSince !== since) return false;
  data.findings = data.findings.concat(body.findings);
  data.findingsTotal = body.total;
  return body.findings.length > 0;
}

async function loadSessions() {
  const body = await fetchFresh("/api/sessions");
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

async function poll() {
  const since = sinceFor();
  const jobs = [loadRules(), loadActivity(since, "activity"), loadFindings(since), loadSessions()];
  jobs.push(since === 0 ? Promise.resolve(false) : loadActivity(0, "activityAll"));
  if (ui.selectedSession && ui.view === "sessions") jobs.push(loadSession(ui.selectedSession));
  try {
    const results = await Promise.all(jobs);
    if (since === 0) data.activityAll = data.activity;
    data.updatedAt = Date.now();
    data.disconnected = false;
    data.loaded = true;
    backoff = INTERVAL;
    onChange(results.some(Boolean));
  } catch (error) {
    data.disconnected = true;
    backoff = Math.min(MAX_BACKOFF, backoff * 2);
    console.warn("poll failed", error);
    onChange(false);
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

export async function loadRepo() {
  try {
    const { body } = await get("/api/rule-context");
    data.repo = body.context.projectName;
  } catch { data.repo = ""; }
}
