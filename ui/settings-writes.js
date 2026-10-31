/**
 * The page's rule setting writes: rules on or off, a whole rule pack on or off, and whether a rule pack's rules honour
 * linter suppression comments. Each change is one `PUT /api/repository/settings` per folder and setting it writes,
 * owned by this worktree or the repository (`owner()`), sent in click order. The page shows the change at once; after a
 * failed write it puts back what nothing saved and reads the rules in full, so it ends on what the daemon holds.
 * `toast` and `invalidate` are the page's.
 */
import { send } from "./api.js";
import { data } from "./state.js";
import { inPack, packSettingDecides } from "./derive.js";
import { captureScope, inScope, refetch, refreshNow } from "./poll.js";
import { failed as failedText } from "./format.js";

const SETTINGS = "/api/repository/settings";
/** A failed write's toast when the action names no failure of its own. */
const SAVE_FAILED = "Couldn’t save";

/**
 * How long one setting write waits for the daemon. A write it never answers then fails like any other, so the writes
 * clicked after it for the same repository go out.
 */
const WRITE_TIMEOUT = 30_000;

/** One setting write, given up after `WRITE_TIMEOUT`. */
async function put(body) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(new Error(`the daemon did not answer within ${WRITE_TIMEOUT / 1000} s`)), WRITE_TIMEOUT);
  try {
    return await send("PUT", SETTINGS, body, { signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

/** One entry per folder: the targets written there. */
function byFolder(pairs) {
  const folders = new Map();
  for (const [folder, target] of pairs) folders.set(folder, [...(folders.get(folder) ?? []), target]);
  return folders;
}

/** One write per folder of `folders` (folder → targets), each setting `setting`. */
const writesOf = (folders, setting) => [...folders].map(([folder, targets]) => ({ folder, targets, setting }));

const storedRules = ids => ids.map(id => data.rules.get(id)).filter(r => r !== undefined);
const ruleTargets = rules => byFolder(rules.flatMap(r => r.folders.map(folder => [folder, { kind: "rule", id: r.rule.id }])));

/** The folders pack `packId` is added at whose folder is known (one still loading for an add is not written yet). */
const packFolders = packId => (data.attachedPacks.get(packId) ?? []).filter(folder => typeof folder === "string");

/**
 * The writes that switch every rule of pack `packId` on or off: at each folder the pack is added at, the pack's own
 * setting, then its rules' own on cleared (`on: null`) for the same owner and folder. A rule's own setting outranks its
 * pack's at one owner and folder, so without the clearing a rule switched on its own would keep its old state; cleared,
 * one pack setting per folder decides them all, and a rule the pack gains later follows it too. A cleared rule keeps
 * its other values (mode, suppressions), and one with no setting stays without one.
 */
export function packSwitchWrites(packId, enabled) {
  const rules = [...data.rules.values()].filter(r => inPack(r, packId));
  return packFolders(packId).flatMap(folder => {
    // The rules the pack added at this folder (a rule may also be added on its own elsewhere).
    const own = rules.filter(r => r.folderStates.some(state => state.folder === folder && state.source?.kind === "pack" && state.source.id === packId))
      .map(r => ({ kind: "rule", id: r.rule.id }));
    const pack = { folder, targets: [{ kind: "pack", id: packId }], setting: { on: enabled } };
    return own.length === 0 ? [pack] : [pack, { folder, targets: own, setting: { on: null } }];
  });
}

/**
 * The last setting write queued per repository and worktree the page has shown (keyed by its target): each is sent
 * once the one clicked before it for the same target is answered, in click order, even after the page has shown
 * another repository and come back. Another target's writes never wait on these.
 */
const writing = new Map();

export function settingWrites({ toast, invalidate, owner }) {
  /**
   * Send `puts` (`{ folder, targets, setting }` each) in order, and show the change on `rules` at once with `show`.
   * The repository, owner and rows are fixed when the action starts: switching repository mid-save writes nothing there.
   * `label` is the toast once saved; `failed` begins the toast when a write fails, before the error.
   */
  async function save(puts, rules, show, label, failed = SAVE_FAILED) {
    if (puts.length === 0) return;
    const scope = captureScope();
    const by = owner();
    const before = new Map(rules.map(r => [r, { ...r }]));
    for (const r of rules) show(r);
    const shown = new Map(rules.map(r => [r, { ...r }]));
    invalidate();
    const key = JSON.stringify(scope.target);
    const previous = writing.get(key) ?? Promise.resolve();
    const { promise: settled, resolve: done } = Promise.withResolvers();
    writing.set(key, settled);
    let failure = null, saved = 0;
    try {
      await previous;
      for (const { folder, targets, setting } of puts) {
        if (!inScope(scope)) break;
        await put({ ...scope.target, folder, owner: by, targets, ...setting });
        saved += 1;
      }
    } catch (error) {
      failure = error;
    } finally {
      done();
    }
    // Another repository's page now: these rows and the outcome are not its.
    if (!inScope(scope)) return;
    if (failure === null) {
      toast(label);
    } else {
      // A write that saved may decide folders below it: only the reload says what holds where.
      if (saved === 0) for (const r of rules) undo(r, before.get(r), shown.get(r));
      refetch("rules");
      toast(failedText(failed, failure));
    }
    invalidate();
    refreshNow();
  }

  /** Switch rules on or off, at every folder each is added at. */
  function setEnabled(ids, enabled, label, failed) {
    const rules = storedRules(ids);
    return save(writesOf(ruleTargets(rules), { on: enabled }), rules, r => { r.enabled = enabled; }, label, failed);
  }

  /** Switch every rule of pack `packId` on or off (`packSwitchWrites`). */
  function setPackEnabled(packId, enabled, label, failed) {
    const rules = [...data.rules.values()].filter(r => inPack(r, packId));
    return save(packSwitchWrites(packId, enabled), rules, r => { r.enabled = enabled; }, label, failed);
  }

  /**
   * Honour or ignore suppression comments for a rule pack, at every folder it is added at. The page shows it in the
   * folders of the pack's rules its setting decides: not where a setting that outranks it decides, by owner, folder
   * depth, or a rule's own setting at the same owner and folder.
   */
  function setPackSuppressions(packId, suppressions, label, failed) {
    const by = owner();
    const decides = state => packSettingDecides(state, packId, by, data.checkout?.checkoutPath);
    const rules = [...data.rules.values()].filter(r => r.folderStates.some(decides));
    const folders = byFolder(packFolders(packId).map(folder => [folder, { kind: "pack", id: packId }]));
    return save(writesOf(folders, { suppressions }), rules, showSuppressions(suppressions, decides, decider(by, "pack")), label, failed);
  }

  return { setEnabled, setPackEnabled, setPackSuppressions };
}

/**
 * Put back on stored rule `r` what a write that saved nothing showed (`shown`, over `was`), except wherever a later
 * change has replaced it since.
 */
function undo(r, was, shown) {
  r.folderStates = r.folderStates.map((state, index) => (state === shown.folderStates[index] ? was.folderStates[index] : state));
  r.suppressions = r.folderStates[0]?.suppressions ?? r.suppressions;
  r.suppressionsSource = r.folderStates[0]?.suppressionsSource ?? r.suppressionsSource;
  if (r.enabled === shown.enabled) r.enabled = was.enabled;
}

/**
 * What decides a folder's suppressions once a setting on `target` written as `owner` lands there, as the API reports
 * it, so a second change before the page refreshes weighs the first.
 */
const decider = (owner, target) => state => (owner === "checkout"
  ? { by: "checkout", checkoutPath: data.checkout?.checkoutPath, folder: state.folder, target }
  : { by: "repository", folder: state.folder, target });

/**
 * Show a suppressions change on a stored rule, in every folder it is added at that `changes`, with what now decides it
 * (`source`, from the folder's state).
 */
const showSuppressions = (suppressions, changes, source) => r => {
  r.folderStates = r.folderStates.map(state => (changes(state) ? { ...state, suppressions, suppressionsSource: source(state) } : state));
  r.suppressions = r.folderStates[0]?.suppressions ?? suppressions;
  r.suppressionsSource = r.folderStates[0]?.suppressionsSource ?? r.suppressionsSource;
};
