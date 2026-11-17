/**
 * The selected session's detail (`data.sessionDetail`): its head, its transcript and its violations. A poll refreshes
 * the head and appends the events recorded since; Load earlier reaches further back, through the transcript and the
 * violations together. A session's loads run one at a time, and a session the daemon no longer has is forgotten.
 * Each fresh head is written over the session's row in the loaded list, so a row past the list's first page stays
 * current too.
 */
import { get } from "./api.js";
import { data, ui, pagedList } from "./state.js";
import { repositoryFilter } from "./scoped-requests.js";
import { loadFirstPage, loadNextPage } from "./paged-lists.js";

/** Page sizes: the selected session's violations, its transcript. */
const SESSION_FINDINGS_PAGE = 200;
const EVENTS_PAGE = 500;

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
 * The session's head (`GET /api/sessions/<id>`), or `null` once it has been dropped: a session missing on its first
 * load fails with its 404, while one that disappears later (a reset) is forgotten.
 */
async function sessionHead(id, detail) {
  try {
    const { body } = await get(`/api/sessions/${encodeURIComponent(id)}`);
    return body;
  } catch (error) {
    if (!detail.loaded || !/answered 404$/u.test(error.message)) throw error;
    dropSession(id);
    return null;
  }
}

/** Takes a fresh head and its gaps into `detail` and over the session's list row. Whether either changed. */
function takeHead(detail, body) {
  const was = detail.head, wasGaps = detail.gaps;
  detail.head = body.session;
  detail.gaps = body.gaps;
  patchSessionRow(body.session);
  return was?.eventCount !== body.session.eventCount || was?.endedAt !== body.session.endedAt
    || JSON.stringify(wasGaps) !== JSON.stringify(body.gaps);
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
    const body = await sessionHead(id, detail);
    if (body === null) return true;
    const changed = takeHead(detail, body);
    const page = await eventsPage(id, detail.loaded ? `after=${detail.nextAfter}` : `before=${Number.MAX_SAFE_INTEGER}`);
    if (!detail.loaded) detail.earlier = page.events.length === EVENTS_PAGE;
    detail.loaded = true;
    detail.nextAfter = page.nextAfter;
    if (page.events.length === 0) return changed;
    detail.events = detail.events.concat(page.events);
    return true;
  });
}

/** Whether Load earlier pages the session's violations next rather than its transcript: they reach back less far. */
function violationsNext(id, detail) {
  const slot = data.sessionFindings;
  const oldestViolation = slot.rows.at(-1)?.at ?? Infinity;
  const oldestEvent = detail.earlier ? detail.events[0]?.observedAt ?? -Infinity : -Infinity;
  return slot.owner === id && slot.next !== null && oldestViolation >= oldestEvent;
}

/** The transcript's page before its oldest event loaded, prepended. */
function loadEarlierEvents(id, detail) {
  return sessionLoad(detail, async () => {
    const page = await eventsPage(id, `before=${detail.events[0]?.sequence ?? Number.MAX_SAFE_INTEGER}`);
    detail.earlier = page.events.length === EVENTS_PAGE;
    detail.events = page.events.concat(detail.events);
    return page.events.length > 0;
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
  if (violationsNext(id, detail)) return loadNextPage("sessionFindings", "violations");
  if (!detail.earlier) return false;
  return loadEarlierEvents(id, detail);
}
