/**
 * The page's rule pack writes: adding and removing packs, and setting a repository up with them. Each is one
 * `PUT /api/repository/attachments` to the repository the click was in, queued in click order (`queueInScope`); the
 * page shows its outcome at once and rolls it back if it fails. `toast` and `invalidate` are the page's.
 */
import { data, ui, packById, packFolder, isSetUp, keptPacks } from "./state.js";
import { recommendationsFor, captureScope, inScope, queueInScope, refreshNow } from "./poll.js";
import { andList, failed as failedText } from "./format.js";

const ATTACHMENTS = "/api/repository/attachments";

export const packName = id => packById(id)?.name ?? id;
const packAttachment = (id, folder) => ({ sourceKind: "pack", sourceId: id, folder });

/**
 * The recommendations for the scope a click was in: the page's own when it has them, else fetched for that scope,
 * so a switch while they load never lends the write another repository's folders.
 */
function recommendedFor(scope) {
  const loaded = data.recommendations;
  return loaded === null
    ? recommendationsFor(scope).catch(error => { throw new Error(`couldn’t read the repo to place it (${error.message})`); })
    : Promise.resolve(loaded);
}

/** After set-up: the packs it added, by name (their folders show on each pack page). */
function setUpMessage(added) {
  const repo = data.repository?.name ?? "This repo";
  return added.length === 0 ? `${repo} is set up, but no rule pack fits it yet.`
    : `${repo} is set up with ${andList(added.map(({ sourceId }) => packName(sourceId)))}.`;
}

export function attachmentWrites({ toast, invalidate }) {
  /** True while the page shows the repository the write was started in; else the write is dropped, and said so. */
  function stillShown(scope) {
    if (inScope(scope)) return true;
    toast("Nothing saved: you switched repos before it was sent.");
    return false;
  }

  /**
   * Packs added by one click, decided at the click so the write keeps its place in the queue: each pack's folder for
   * the shown state, and a promise of the attachments to write (`null` once the page shows another repository). While
   * the recommendations load, a folder is a promise of it; if they cannot be read, the write fails and the shown
   * state is rolled back.
   */
  function additions(scope, ids) {
    const loaded = data.recommendations;
    const recommendations = recommendedFor(scope);
    return {
      folders: ids.map(id => (loaded === null ? recommendations.then(r => packFolder(r, id), () => "") : packFolder(loaded, id))),
      add: recommendations.then(r => (stillShown(scope) ? ids.map(id => packAttachment(id, packFolder(r, id))) : null)),
    };
  }

  /**
   * One write to the repository the action started in, queued at the click behind the attachment writes clicked
   * before it; `body` may still be loading. The shown packs change to `attached` (and a `configure` write marks the
   * repository set up): at once, or when `attached` is a promise, once it resolves. On error they go back to what
   * they were before this click and the poll reads them back; the writes clicked after it are dropped, so that also
   * undoes their changes. Dropped when the page shows another repository before it is sent; once it is, neither the
   * answer nor a rollback touches the page. `failed` begins the toast when the write fails, before the error.
   */
  async function setAttachments(scope, { body, attached, configure = false }, message, failed = "Couldn’t save") {
    const before = { attached: data.attachedPacks, repository: data.repository };
    const show = packs => {
      if (packs === null || !inScope(scope)) return;
      data.attachedPacks = packs;
      if (configure && data.repository !== null) data.repository = { ...data.repository, setUp: true };
      invalidate();
    };
    ui.removeAsk = null;
    if (attached instanceof Map) show(attached); else attached.then(show, () => {});
    invalidate();
    let shown;
    try {
      shown = await queueInScope(scope, "PUT", ATTACHMENTS, body);
    } catch (error) {
      if (!inScope(scope)) return;
      data.attachedPacks = before.attached;
      data.repository = before.repository;
      toast(failedText(failed, error));
      await refreshNow();
      return;
    }
    if (!shown) return;
    await refreshNow();
    toast(typeof message === "function" ? message() : message);
  }

  /** Add packs, each at its recommended folder, else the whole repository. The first addition sets the repository up. */
  function addPacks(ids, message, failed) {
    const fresh = ids.filter(id => !data.attachedPacks.has(id));
    if (fresh.length === 0) return;
    const scope = captureScope();
    const { folders, add } = additions(scope, fresh);
    const attached = new Map([...data.attachedPacks, ...fresh.map((id, i) => [id, [folders[i]]])]);
    return setAttachments(scope, { body: add.then(a => a && { add: a }), attached }, message, failed);
  }

  /**
   * Remove a pack: every attachment it has, at each folder it is added at, in one write. A folder still loading for
   * an add queued before this click is awaited, and that add is sent first.
   */
  function removePack(id, message, failed) {
    const folders = data.attachedPacks.get(id) ?? [];
    const attached = new Map([...data.attachedPacks].filter(([packId]) => packId !== id));
    const body = Promise.all(folders).then(known => ({ remove: known.map(folder => packAttachment(id, folder)) }));
    return setAttachments(captureScope(), { body, attached }, message, failed);
  }

  /** Review in a repository that is not set up: the kept packs at their folders, and set up, in one write. */
  function setUpWithPacks(ids, message, failed) {
    const scope = captureScope();
    const { folders, add } = additions(scope, ids);
    const attached = new Map(ids.map((id, i) => [id, [folders[i]]]));
    return setAttachments(scope, { body: add.then(a => a && { add: a, configure: true }), attached, configure: true }, message, failed);
  }

  /**
   * "Set up with recommended packs": the packs recommended for this checkout, each added at its folder, and set up,
   * in one write that takes its place among the attachment writes at the click. The page shows them added as soon
   * as they are known (at once when it has them), so a change clicked after it is a single-pack write queued
   * behind it rather than a second set-up it would undo.
   */
  async function setUpRecommended() {
    if (ui.settingUp) return;
    ui.settingUp = true;
    const scope = captureScope();
    const loaded = data.recommendations;
    const recommendations = recommendedFor(scope);
    const asAttached = r => new Map([...r.map(({ packId, folder }) => [packId, [folder]]), ...data.attachedPacks]);
    let added = [];
    const body = recommendations.then(r => {
      if (!stillShown(scope)) return null;
      added = r.map(({ packId, folder }) => packAttachment(packId, folder));
      return { add: added, configure: true };
    });
    const attached = loaded === null ? recommendations.then(asAttached) : asAttached(loaded);
    await setAttachments(scope, { body, attached, configure: true }, () => setUpMessage(added), "Couldn’t set up the repo");
    ui.settingUp = false;
    invalidate();
  }

  /**
   * Keep a pack on or off: from the review's switch, or from its panel during the review. While the repository is
   * not set up, the change writes the whole kept set (`keptPacks()`, the recommended packs until then) with that pack
   * in or out, and sets it up, so the packs kept alongside it stay; once it is, it adds or removes that pack alone.
   */
  function keepPack(id, keep, message, failed) {
    if (data.repository === null) return;
    const kept = keptPacks();
    if (kept.includes(id) === keep) return;
    if (!isSetUp()) return setUpWithPacks(keep ? [...kept, id] : kept.filter(x => x !== id), message, failed);
    return keep ? addPacks([id], message, failed) : removePack(id, message, failed);
  }

  return { addPacks, removePack, setUpRecommended, keepPack };
}
