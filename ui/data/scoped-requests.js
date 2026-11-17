/**
 * The requests that belong to the repository the page shows. A scope `generation` is bumped whenever the page shows
 * another repository (`forgetScope`), so an answer to a request sent for the previous one lands nowhere. Reads are
 * conditional GETs with an ETag per data slot (`fetchFresh`); writes are fixed to the scope they were clicked in
 * (`captureScope`) and touch the page only while it still shows that scope. `poll.js` and the loaders it runs read
 * through here; only this module assigns `generation`.
 */
import { get, send } from "./api.js";
import { data, ui } from "./state.js";

/**
 * ETag per data slot, not per URL: range-scoped resources fetch a different URL for every range,
 * and the tag must describe what `data.*` currently holds so that returning to a range whose body
 * the daemon already served once still answers 200 instead of a 304 that leaves the other range's
 * data in place.
 */
const etags = new Map();
/** Bumped when the scope changes, so a response for the previous repository lands nowhere. */
export let generation = 0;

/** The page shows another scope: what is in flight for the previous one lands nowhere, and no ETag of its data holds. */
export function forgetScope() {
  generation += 1;
  etags.clear();
}

/**
 * Conditional GET; resolves `undefined` on 304, after a scope change, or once `wanted()` says the answer no longer
 * belongs where it would land (its ETag is then not kept either); the body on 200. `slot` names the ETag's data slot.
 */
export async function fetchFresh(path, slot = path, wanted = () => true) {
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

export const scopeQuery = () => new URLSearchParams(scopeTarget()).toString();

/** `&repository=<key>` for the machine-wide reads (activity, violations, sessions), once the key is known. */
export function repositoryFilter() {
  const key = data.repository?.repositoryKey ?? ui.scope?.repository;
  return key === undefined ? "" : `&repository=${encodeURIComponent(key)}`;
}
