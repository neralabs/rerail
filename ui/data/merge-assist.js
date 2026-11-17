/**
 * Merge Assist's reads and writes for the scoped checkout (`data.mergeAssist`). A read runs at most once at a time,
 * and the sidebar's count reads it at most every 30 s. Every write answers the checkout's status, which replaces the
 * page's unless a later write was sent, and a read sent before a write answered is dropped, since it may hold the
 * state from before that write. A failed read is kept for the page and never fails the poll: the rest of the
 * dashboard does not depend on Merge Assist.
 */
import { send } from "./api.js";
import { data } from "./state.js";
import { fetchFresh, generation, inScope, refetch, scopeQuery } from "./scoped-requests.js";

/** How long the sidebar's Merge Assist count may be before it is read again, on a page other than Merge Assist's. */
export const MERGE_ASSIST_COUNT_EVERY = 30_000;
/**
 * The last Merge Assist read: when it last answered, an error included (so the sidebar's count backs off to every
 * 30 s while the read fails), and the one in flight with the scope generation it was sent for.
 */
let mergeAssistRead = { at: 0, loading: null, generation: -1 };
/**
 * Merge Assist writes sent and answered. A read in flight while either count moves may hold the checkout's state from
 * before a write, so it lands nowhere; and only the latest write sent replaces the status, so an earlier write that
 * answers last cannot put back what a later one changed.
 */
const mergeAssistWrites = { sent: 0, answered: 0 };
/**
 * Merge Assist requests sent, reads and writes alike, numbered in order (`asked`), and the number of the one whose
 * answer `data.mergeAssist` holds (`held`): a conflict link is looked up only in a status asked for after it.
 */
const mergeAssistRequests = { asked: 0, held: 0 };

/** The page shows another scope: no read answered for it yet, and none in flight. */
export function forgetMergeAssistRead() {
  mergeAssistRead = { at: 0, loading: null, generation: -1 };
}

/** The number of the last Merge Assist request sent, for `mergeAssistReadAfter`. */
export const mergeAssistMark = () => mergeAssistRequests.asked;

/**
 * Whether `data.mergeAssist` answers a request sent after `mark` (`mergeAssistMark()`): not one read before it, which
 * the sidebar's count may have read half a minute ago, nor one already out then.
 */
export const mergeAssistReadAfter = mark => data.mergeAssist !== null && mergeAssistRequests.held > mark;

/**
 * Merge Assist read `request` answered: `body`, or nothing new on `undefined` (304), which confirms the status held.
 * Whether anything shown changed.
 */
function takeMergeAssist(body, request) {
  mergeAssistRead.at = Date.now();
  mergeAssistRequests.held = request;
  const changed = data.mergeAssistError !== null || data.mergeAssistExperiment !== true;
  Object.assign(data, { mergeAssistError: null, mergeAssistExperiment: true });
  if (body === undefined) return changed;
  data.mergeAssist = body;
  return true;
}

/**
 * A Merge Assist read failed with `error`, kept for the page. It says nothing of whether the daemon runs Merge Assist:
 * only a read that answers shows its sidebar item. Whether anything shown changed.
 */
function takeMergeAssistFailure(error) {
  mergeAssistRead.at = Date.now();
  // The API's own reason when it answered one ("Rerail does not know the checkout …"), else the status line.
  const why = error.apiMessage ?? error.message;
  const changed = data.mergeAssistError !== why;
  data.mergeAssistError = why;
  return changed;
}

/** The daemon runs without Merge Assist (`404 experimental-off`): nothing of it is shown or read again until it restarts. */
function experimentOff(error) {
  if (error.status !== 404 || error.code !== "experimental-off") return false;
  Object.assign(data, { mergeAssist: null, mergeAssistError: null, mergeAssistExperiment: false });
  return true;
}

/** Whether a Merge Assist read is skipped: the daemon runs without it, or the last answer is younger than `maxAge`. */
function skipMergeAssist(maxAge) {
  if (data.mergeAssistExperiment === false) return true;
  const held = data.mergeAssist !== null || data.mergeAssistError !== null;
  return maxAge > 0 && held && Date.now() - mergeAssistRead.at < maxAge;
}

/** Read Merge Assist and take the answer while `current()` says it still lands. Whether anything shown changed. */
async function readMergeAssist(current) {
  const request = ++mergeAssistRequests.asked;
  try {
    const body = await fetchFresh(`/api/merge-assist?${scopeQuery()}`, "mergeAssist", current);
    return current() && takeMergeAssist(body, request);
  } catch (error) {
    if (error.name === "AccessRequired") throw error;
    return experimentOff(error) || (current() && takeMergeAssistFailure(error));
  }
}

/**
 * Merge Assist for the scoped checkout (`data.mergeAssist`), at most once at a time; `maxAge` skips the read while the
 * last answer is younger. A failure is kept in `data.mergeAssistError` for the page and the sidebar, never failing
 * the poll: the rest of the dashboard does not depend on Merge Assist. A `404 experimental-off` is no failure: the
 * daemon runs without Merge Assist (`data.mergeAssistExperiment` false), and nothing reads it again until it restarts.
 */
export function loadMergeAssist(maxAge = 0) {
  if (mergeAssistRead.loading !== null && mergeAssistRead.generation === generation) return mergeAssistRead.loading;
  if (skipMergeAssist(maxAge)) return false;
  const started = generation;
  const { sent, answered } = mergeAssistWrites;
  const current = () => started === generation && sent === mergeAssistWrites.sent && answered === mergeAssistWrites.answered;
  const loading = readMergeAssist(current).finally(() => {
    if (mergeAssistRead.loading === loading) mergeAssistRead.loading = null;
  });
  mergeAssistRead = { at: mergeAssistRead.at, loading, generation: started };
  return loading;
}

/**
 * A write's answered status replaces `data.mergeAssist`, unless a later write was sent meanwhile (`write` is its count;
 * `request` its number among all Merge Assist requests).
 */
function keepWriteStatus(write, request, status) {
  if (status === undefined || write !== mergeAssistWrites.sent) return;
  mergeAssistRequests.held = request;
  data.mergeAssist = status;
  data.mergeAssistError = null;
  mergeAssistRead.at = Date.now();
}

/**
 * One Merge Assist write to a captured scope's target. Every Merge Assist write answers the checkout's status: it
 * replaces `data.mergeAssist` unless a later write was sent meanwhile, and a read sent before it answered lands nowhere.
 * Resolves that status, or `null` when there is none or the page shows another repository by then. Rejects with the
 * write's error only while the page still shows the scope.
 */
export async function writeMergeAssist(scope, method, path, body) {
  const write = ++mergeAssistWrites.sent, request = ++mergeAssistRequests.asked;
  try {
    const { body: status } = await send(method, path, { ...scope.target, ...body });
    if (!inScope(scope)) return null;
    keepWriteStatus(write, request, status);
    return status ?? null;
  } catch (error) {
    if (inScope(scope)) throw error;
    return null;
  } finally {
    // A write to a scope no longer shown has nothing to say about the reads of the one shown now.
    if (inScope(scope)) mergeAssistWrites.answered += 1;
    // The slot's ETag describes the last read, not what the write put in its place.
    refetch("mergeAssist");
  }
}
