/**
 * The page's rule setting writes: a rule on or off, and whether a rule or a rule pack honours linter suppression
 * comments. Each change is one `PUT /api/repository/settings` per folder it is written at, owned by this worktree or
 * the repository (`owner()`), sent in click order. The page shows the change at once; after a failed write it puts
 * back what nothing saved and reads the rules in full, so it ends on what the daemon holds. `toast` and `invalidate`
 * are the page's.
 */
import { send } from "./api.js";
import { data } from "./state.js";
import { packSettingDecides, settingDecides } from "./derive.js";
import { captureScope, inScope, refetch, refreshNow } from "./poll.js";

const SETTINGS = "/api/repository/settings";

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

const storedRules = ids => ids.map(id => data.rules.get(id)).filter(r => r !== undefined);
const ruleTargets = rules => byFolder(rules.flatMap(r => r.folders.map(folder => [folder, { kind: "rule", id: r.rule.id }])));

/**
 * The last setting write queued per repository and worktree the page has shown (keyed by its target): each is sent
 * once the one clicked before it for the same target is answered, in click order, even after the page has shown
 * another repository and come back. Another target's writes never wait on these.
 */
const writing = new Map();

export function settingWrites({ toast, invalidate, owner }) {
  /**
   * Write `setting` at each folder of `folders` for its targets, and show it on `rules` at once with `show`.
   * The repository, owner and rows are fixed when the action starts: switching repository mid-save writes nothing there.
   */
  async function save(folders, setting, rules, show, label) {
    if (folders.size === 0) return;
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
      for (const [folder, targets] of folders) {
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
      // A folder that saved may decide others below it: only the reload says what holds where.
      if (saved === 0) for (const r of rules) undo(r, before.get(r), shown.get(r));
      refetch("rules");
      toast(`Could not save: ${failure.message}`);
    }
    invalidate();
    refreshNow();
  }

  /** Switch rules on or off, at every folder each is added at. */
  function setEnabled(ids, enabled, label) {
    const rules = storedRules(ids);
    return save(ruleTargets(rules), { on: enabled }, rules, r => { r.enabled = enabled; }, label);
  }

  /**
   * Honour or ignore suppression comments for rules, at every folder each is added at. The page shows it in the
   * folders the rule's setting decides: not where a setting that outranks it decides, by owner or folder depth.
   */
  function setSuppressions(ids, suppressions, label) {
    const rules = storedRules(ids);
    const by = owner();
    const decides = state => settingDecides(state, "rule", by, data.checkout?.checkoutPath);
    return save(ruleTargets(rules), { suppressions }, rules, showSuppressions(suppressions, decides, decider(by, "rule")), label);
  }

  /**
   * Honour or ignore suppression comments for a rule pack, at every folder it is added at. The page shows it in the
   * folders of the pack's rules its setting decides: not where a setting that outranks it decides, by owner, folder
   * depth, or a rule's own setting at the same owner and folder.
   */
  function setPackSuppressions(packId, suppressions, label) {
    const by = owner();
    const decides = state => packSettingDecides(state, packId, by, data.checkout?.checkoutPath);
    const rules = [...data.rules.values()].filter(r => r.folderStates.some(decides));
    const folders = byFolder((data.attachedPacks.get(packId) ?? []).map(folder => [folder, { kind: "pack", id: packId }]));
    return save(folders, { suppressions }, rules, showSuppressions(suppressions, decides, decider(by, "pack")), label);
  }

  return { setEnabled, setSuppressions, setPackSuppressions };
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
 * Show a suppressions change on a stored rule, in every folder it is added at that `changes` (all by default), with
 * what now decides it (`source`, from the folder's state).
 */
const showSuppressions = (suppressions, changes = () => true, source = state => state.suppressionsSource) => r => {
  r.folderStates = r.folderStates.map(state => (changes(state) ? { ...state, suppressions, suppressionsSource: source(state) } : state));
  r.suppressions = r.folderStates[0]?.suppressions ?? suppressions;
  r.suppressionsSource = r.folderStates[0]?.suppressionsSource ?? r.suppressionsSource;
};
