/**
 * The page's rule setting writes: a rule on or off, and whether a rule or a rule pack honours linter suppression
 * comments. Each change is one `PUT /api/repository/settings` per folder it is written at, owned by this worktree or
 * the repository (`owner()`). The page shows the change at once and rolls it back if a write fails; the poll that
 * follows reads back the sources. `toast` and `invalidate` are the page's.
 */
import { send } from "./api.js";
import { data } from "./state.js";
import { followsPack } from "./derive.js";
import { captureScope, inScope, refreshNow } from "./poll.js";

const SETTINGS = "/api/repository/settings";

/** One entry per folder: the targets written there. */
function byFolder(pairs) {
  const folders = new Map();
  for (const [folder, target] of pairs) folders.set(folder, [...(folders.get(folder) ?? []), target]);
  return folders;
}

const storedRules = ids => ids.map(id => data.rules.get(id)).filter(r => r !== undefined);
const ruleTargets = rules => byFolder(rules.flatMap(r => r.folders.map(folder => [folder, { kind: "rule", id: r.rule.id }])));

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
    invalidate();
    let failure = null;
    try {
      for (const [folder, targets] of folders) {
        await send("PUT", SETTINGS, { ...scope.target, folder, owner: by, targets, ...setting });
      }
    } catch (error) {
      failure = error;
    }
    // Another repository's page now: these rows and the outcome are not its.
    if (!inScope(scope)) return;
    if (failure === null) {
      toast(label);
    } else {
      for (const [r, was] of before) Object.assign(r, was);
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

  /** Honour or ignore suppression comments for rules, at every folder each is added at. */
  function setSuppressions(ids, suppressions, label) {
    const rules = storedRules(ids);
    return save(ruleTargets(rules), { suppressions }, rules, showSuppressions(suppressions), label);
  }

  /**
   * Honour or ignore suppression comments for a rule pack, at every folder it is added at. The page shows it on the
   * pack's rules that follow their pack: not on a rule with a setting of its own, nor on one this worktree decides
   * when the change is the repository's.
   */
  function setPackSuppressions(packId, suppressions, label) {
    const by = owner();
    const rules = [...data.rules.values()].filter(r => r.source.kind === "pack" && r.source.id === packId && followsPack(r, by));
    const folders = byFolder((data.attachedPacks.get(packId) ?? []).map(folder => [folder, { kind: "pack", id: packId }]));
    return save(folders, { suppressions }, rules, showSuppressions(suppressions), label);
  }

  return { setEnabled, setSuppressions, setPackSuppressions };
}

/** Show a suppressions change on a stored rule, in every folder it is added at. */
const showSuppressions = suppressions => r => {
  r.suppressions = suppressions;
  r.folderStates = r.folderStates.map(state => ({ ...state, suppressions }));
};
