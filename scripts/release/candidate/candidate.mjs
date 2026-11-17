// The release candidate: the commit a release is built, tested, tagged and published from, and the Release pull
// request that merging releases it (docs/release.md, "Freezing a release").
//
// Two commits, because they answer different questions:
//   - the candidate: a base commit, the commits cherry-picked onto it, and the Release commit release-version.mjs
//     writes (versions, changelogs, the consumed .changelog/ and .changeset/ entries). It is what gets tagged.
//   - the Release pull request's head: the same Release commit applied to main's tip, so merging it is always a clean
//     squash that brings main the versions and changelogs. Merging the candidate itself would conflict on every entry
//     main reworded after the freeze, cherry-picked or not.
// Not frozen, the release follows main: the base is main's tip, so the two are one commit, rebuilt on every push.
// Frozen, the base and the cherry-picks stay put until someone freezes again, cherry-picks or unfreezes. Either way
// the version core releases as can be set by hand (set-version), and holds until the release merges.
//
// The state is read from one place, the Release pull request's head commit: its message names the candidate
// (`Release-Candidate`), whose own message names the base and the cherry-picks. release-candidate.mjs is the command.
//
// It imports nothing that needs `npm ci`: release-packs.yml reads it before installing anything.
import { execFileSync } from "node:child_process";
import { compareVersions } from "../dist-tags.mjs";

/** The Release pull request's branch. */
export const RELEASE_BRANCH = "changeset-release/main";
/** The frozen candidate's branch: it exists only while a release is frozen. */
export const CANDIDATE_BRANCH = "changeset-release/candidate";
/** The file whose presence at a commit says the commit's own workflows can release a candidate. */
export const FLOW_MARKER = "scripts/release/candidate/candidate.mjs";

const SUBJECT = "Release";
const BASE = "Release-Base";
const PICK = "Release-Cherry-Pick";
const CANDIDATE = "Release-Candidate";
const VERSION = "Release-Version";
/** Where the remote's main, release branches and tags are fetched to, so a stale local branch is never read. */
const REFS = "refs/release";
const MAIN_REF = `${REFS}/main`;
const RELEASE_REF = `${REFS}/${RELEASE_BRANCH}`;
const CANDIDATE_REF = `${REFS}/${CANDIDATE_BRANCH}`;
/** The entries a Release commit consumes: one main changed after the freeze is dropped, not a conflict. */
const CONSUMED = [".changelog/", ".changeset/"];

export const short = (sha) => sha.slice(0, 9);
const REBUILD = "Rebuild it with `gh workflow run release-pr.yml -f action=freeze` (or `-f action=unfreeze` to follow main again).";

/** The tag that starts the packs' release at released commit `sha` (release-packs.yml): one per release. */
export const releaseTrainTag = (sha) => `release-${sha.slice(0, 12)}`;

/**
 * A candidate's Release commit message: where it was cut, what was cherry-picked onto that, and the version core
 * releases as when it was set by hand (the set-version action), which the next rebuild reads back from here.
 */
export function candidateMessage({ base, picks = [], coreVersion = "" }) {
  const version = coreVersion === "" ? [] : [`${VERSION}: ${coreVersion}`];
  return [SUBJECT, "", `${BASE}: ${base}`, ...picks.map((pick) => `${PICK}: ${pick}`), ...version, ""].join("\n");
}

/** The message of a frozen Release pull request's head: the candidate that merging it releases. */
export const pullRequestMessage = (candidate) => [SUBJECT, "", `${CANDIDATE}: ${candidate}`, ""].join("\n");

/**
 * What a Release commit's message says: `{ candidate }` for a frozen Release pull request's head,
 * `{ base, picks, coreVersion }` for a candidate (`coreVersion` empty unless set by hand). Null for any other commit,
 * such as one made by hand or by GitHub's "Update branch".
 */
export function readReleaseMessage(message) {
  const lines = message.split("\n").map((line) => line.trim());
  if (lines[0] !== SUBJECT) return null;
  const values = (key) => lines.map((line) => new RegExp(`^${key}: ([0-9a-f]{40})$`, "u").exec(line)?.[1]).filter((value) => value !== undefined);
  const [candidate] = values(CANDIDATE);
  if (candidate !== undefined) return { candidate };
  const [base] = values(BASE);
  if (base === undefined) return null;
  const coreVersion = lines.map((line) => new RegExp(`^${VERSION}: (\\d+\\.\\d+\\.\\d+)$`, "u").exec(line)?.[1]).find((value) => value !== undefined) ?? "";
  return { base, picks: values(PICK), coreVersion };
}

/**
 * What the `commit` input of release-pr.yml names: `{ pull }` for a pull request number, `{ rev }` for a commit or a
 * release tag, null when it is empty. Anything else throws, so a typo never becomes a branch name git resolves.
 */
export function parseTarget(text = "") {
  const value = text.trim();
  if (value === "") return null;
  const pull = /^#?(\d{1,6})$/u.exec(value)?.[1];
  if (pull !== undefined) return { pull: Number(pull) };
  if (/^[0-9a-f]{7,40}$/iu.test(value) || /^v\d+\.\d+\.\d+$/u.test(value)) return { rev: value.toLowerCase() };
  throw new Error(`"${value}" is not a commit (7 to 40 hex characters), a release tag (v1.2.3) or a pull request number`);
}

/** Why `commit` can't be frozen as the release's base, or null. */
export function freezeRefusal({ commit, onMain, latestRelease, hasFlow }) {
  if (!onMain && !latestRelease) return `${short(commit)} is not on main and is not the latest release. Freeze a commit that is on main, or the latest release's tag to ship a fix on top of it.`;
  if (!hasFlow) return `${short(commit)} is older than the freeze workflow, so the workflows at that commit cannot release it. Freeze a newer commit.`;
  return null;
}

/**
 * Why core can't be released as `version`, set by hand, or null: it is a plain version, the kind the Release pull
 * request makes, and later than `latest`, the latest release (null before the first).
 */
export function coreVersionRefusal(version, latest) {
  if (!/^\d+\.\d+\.\d+$/u.test(version)) return `"${version}" is not a version like 0.2.0. The Release pull request makes plain versions; a beta or next preview is tagged by hand (docs/release.md, "Tags and channels").`;
  if (latest !== null && compareVersions(version, latest) <= 0) return `${version} is not later than the latest release, ${latest}.`;
  return null;
}

/** Why `commit` can't be cherry-picked onto the frozen candidate, or null. */
export function cherryPickRefusal({ commit, frozen, onMain, included }) {
  if (!frozen) return "No release is frozen. Run the freeze action first, then cherry-pick.";
  if (!onMain) return `${short(commit)} is not on main. Merge the fix to main first, then cherry-pick its commit, so main never lacks a fix the release has.`;
  if (included) return `${short(commit)} is already in the frozen release.`;
  return null;
}

/**
 * The verdict on a candidate's own CI, from ci.yml's runs on its commit (`{ created_at, status, conclusion }` each,
 * as the Actions API lists them): `passed` or `failed` by the newest run, `pending` while that has not finished. The
 * runs are read from the workflow, not from check runs, whose names any job with a token can write.
 */
export function candidateCiVerdict(runs) {
  const [latest] = [...runs].sort((a, b) => String(b.created_at ?? "").localeCompare(String(a.created_at ?? "")));
  if (latest === undefined || latest.status !== "completed") return "pending";
  return latest.conclusion === "success" ? "passed" : "failed";
}

/**
 * Why a frozen candidate is not what its Release pull request shows, or null. The pull request's diff is only the
 * Release commit, so a reviewer never sees the candidate's own commits: they must be reviewed commits and nothing
 * else. `baseTrusted`: its base is on main or is the latest release; `picksOnMain`: every cherry-pick it names is on
 * main; `picksMatch`: cherry-picking them onto the base again gives the tree its Release commit sits on;
 * `releaseMatches`: applying its Release commit to the commit the pull request's head sits on gives the head's tree,
 * so what the pull request shows is all the Release commit does.
 */
export function candidateTrustRefusal({ candidate, baseTrusted, picksOnMain, picksMatch, releaseMatches }) {
  const refusals = [
    [!baseTrusted, "its base is not a commit on main or the latest release"],
    [!picksOnMain, "it names a cherry-pick that is not a commit on main"],
    [!picksMatch, "it holds changes that are not its base and the cherry-picks it names"],
    [!releaseMatches, "its Release commit does something this pull request's changes do not show"],
  ];
  const refusal = refusals.find(([refused]) => refused)?.[1];
  return refusal === undefined ? null : `Candidate ${short(candidate)} is not what release-pr.yml builds: ${refusal}. It was pushed to by hand. ${REBUILD}`;
}

/**
 * Why a `release-*` tag's push must not release the packs, or null: the tag is named for its commit
 * (releaseTrainTag), and that commit is one a merged Release pull request released (`released`, their commits).
 * release-pr.yml makes the tag so; this stops one pushed by hand at another commit.
 */
export function releaseTrainRefusal({ tag, commit, released }) {
  if (tag !== releaseTrainTag(commit)) return `${tag} is not the release tag of its commit ${short(commit)}, which is ${releaseTrainTag(commit)}: release-pr.yml makes these tags, when the Release pull request merges.`;
  if (!released.includes(commit)) return `${short(commit)} is not a commit a merged Release pull request released: release-pr.yml makes these tags, when the Release pull request merges.`;
  return null;
}

/**
 * The commit that merging the Release pull request released, given its head commit and that commit's message: the
 * candidate a frozen head names, else the head itself, which followed main. Null for a head this workflow did not
 * make, such as one from before it or one changed by hand, which is never tagged.
 */
export function releasedCommit(head, message) {
  const named = readReleaseMessage(message);
  if (named === null) return null;
  return named.candidate ?? head;
}

/**
 * Why the Release pull request's head can't be merged, or null: merging tags the commit its message names, so the head
 * must be the one release-pr.yml built. `head` is `{ named, parents, parentOnMain, version }` (its message as
 * readReleaseMessage reads it, its parent commits, whether its one parent is on main, its package.json version); frozen,
 * `candidate` is `{ tip, named, commits, version }`: the candidate branch's tip, its message, how many commits it holds
 * past its base, and its package.json version.
 */
export function pullRequestHeadRefusal({ head, candidate }) {
  if (head.named === null) return `This branch's head is not a commit release-pr.yml made: it was pushed to by hand or with "Update branch". ${REBUILD}`;
  if (head.parents.length !== 1 || !head.parentOnMain) return `This branch's head does not sit on a commit of main. ${REBUILD}`;
  return head.named.candidate === undefined ? followingHeadRefusal(head) : frozenHeadRefusal(head, candidate);
}

/** A head that follows main sits on the commit it names as its base. */
function followingHeadRefusal(head) {
  const [parent] = head.parents;
  return head.named.base === parent ? null : `This branch's head names ${short(head.named.base)} as its base but sits on ${short(parent)}. ${REBUILD}`;
}

/** Whether a candidate is its base, one commit for each cherry-pick, and the Release commit. */
const candidateIsBuilt = (candidate) => candidate.named?.base !== undefined && candidate.commits === candidate.named.picks.length + 1;

/** A frozen head names the candidate branch's tip, which release-pr.yml built at the version the head sets. */
function frozenHeadRefusal(head, candidate) {
  const named = short(head.named.candidate);
  const found = candidate.tip === null ? "is gone" : `is at ${short(candidate.tip)}`;
  const refusals = [
    [candidate.tip !== head.named.candidate, `This pull request releases candidate ${named}, but ${CANDIDATE_BRANCH} ${found}.`],
    [!candidateIsBuilt(candidate), `Candidate ${named} is not a base, its cherry-picks and one Release commit: it was pushed to by hand.`],
    [candidate.version !== head.version, `Candidate ${named} is version ${candidate.version}, but this pull request sets ${head.version}.`],
  ];
  const refusal = refusals.find(([refused]) => refused)?.[1];
  return refusal === undefined ? null : `${refusal} ${REBUILD}`;
}

const code = (text) => `\`${text}\``;
const commitLine = ({ commit, subject }) => `${code(short(commit))} ${subject}`;

/**
 * The lines that open a frozen Release pull request's description: what merging it releases, and the three commands
 * that change that. `frozen` is `{ base, baseSubject, picks: [{ commit, subject }] }`.
 */
export function frozenNotice({ base, baseSubject, picks }) {
  const fixes = picks.length === 0 ? [] : ["> ", `> Cherry-picked onto it (${picks.length}):`, ...picks.map((pick) => `> - ${commitLine(pick)}`)];
  return [
    "> [!IMPORTANT]",
    `> **Frozen at ${commitLine({ commit: base, subject: baseSubject })}.** Merging this pull request releases that commit${picks.length === 0 ? "" : " and the fixes below"}, and nothing merged to \`main\` since. Pushes to \`main\` do not change it or clear the checklist.`,
    ...fixes,
    "> ",
    "> - **Add a fix:** merge it to `main`, then `gh workflow run release-pr.yml -f action=cherry-pick -f commit=<commit or pull request number>`.",
    "> - **Freeze a different commit:** `gh workflow run release-pr.yml -f action=freeze -f commit=<commit>`. Fixes it does not hold are kept.",
    "> - **Follow `main` again:** `gh workflow run release-pr.yml -f action=unfreeze`.",
    "> - **Release core as another version:** `gh workflow run release-pr.yml -f action=set-version -f version=<version>`.",
    "> ",
    `> The **Release candidate** check passes once CI passes on the candidate (\`${CANDIDATE_BRANCH}\`), so only a tested commit can be merged.`,
    "",
  ];
}

/**
 * The comment release-pr.yml leaves on the Release pull request after an action, or null when it has nothing to say:
 * what the release now is, where its builds are, and any consumed entry main changed that ships as frozen.
 */
export function actionComment({ action, actor, repository, result }) {
  if (action === "update") return updateComment(result);
  if (!result.changed) return null;
  if (!result.frozen) return followingComment(action, actor, result);
  const runs = (workflow) => `https://github.com/${repository}/actions/workflows/${workflow}?query=branch%3A${encodeURIComponent(CANDIDATE_BRANCH)}`;
  const fixes = result.picks.length === 0 ? "" : ` plus ${result.picks.map((pick) => code(short(pick))).join(", ")}`;
  return [
    `${actionLead(action, actor, result)} The release is now candidate ${code(short(result.candidate))}: base ${code(short(result.base))}${fixes}.`,
    "",
    `Started on the candidate: [CI](${runs("ci.yml")}), the [core build](${runs("release.yml")}) (its \`rerail-package\` artifact is the tarball to test) and the [pack build](${runs("release-packs.yml")}) (\`rule-packs\`). The checklist was cleared, because what merging releases changed.`,
    ...droppedWarning(result),
  ].join("\n");
}

/** What the action did, as the comment's opening sentence. */
function actionLead(action, actor, { coreVersion = "" }) {
  if (action === "set-version") return coreVersion === "" ? `Core's version set back to the next one by @${actor}.` : `Core's version set to ${coreVersion} by @${actor}.`;
  return `${action === "freeze" ? "Frozen" : "Fix cherry-picked"} by @${actor}.`;
}

/** The comment for an action on a release that follows main. */
function followingComment(action, actor, result) {
  if (action === "set-version") return `${actionLead(action, actor, result)} The checklist was cleared, because what merging releases changed.`;
  return `Unfrozen by @${actor}: this pull request follows \`main\` again, and every push to it rewrites the release.`;
}

const pathList = (paths) => paths.map((path) => `- ${code(path)}`);

/** The warning for consumed entries main changed after the freeze, which the release ships as frozen; none, no lines. */
function droppedWarning({ dropped = [] }) {
  if (dropped.length === 0) return [];
  return ["", "`main` changed these entries after the freeze and the change was not cherry-picked, so the release ships them as frozen. Cherry-pick the commit that changed them if the release should have it:", "", ...pathList(dropped)];
}

/**
 * What a push to main has to say on a frozen Release pull request: nothing, unless main changed what the release
 * rewrites. It opens with a marker naming the candidate and the files, so release-pr.yml says it once, not on every
 * later push.
 */
function updateComment(result) {
  const { conflicts = [], dropped = [] } = result;
  if (conflicts.length > 0) {
    return [
      `<!-- release-conflict ${result.candidate} ${conflicts.join(" ")} -->`,
      "**This pull request can't be merged until this is settled.** `main` changed these files after the freeze, and the frozen release changes the same lines:",
      "",
      ...pathList(conflicts),
      "",
      "If the release should have that change, cherry-pick its commit: `gh workflow run release-pr.yml -f action=cherry-pick -f commit=<commit or pull request number>`. If it is for the next release, revert it on `main` until this one is merged, or unfreeze. The frozen release itself is unchanged.",
    ].join("\n");
  }
  if (dropped.length === 0) return null;
  return [`<!-- release-dropped ${result.candidate} ${dropped.join(" ")} -->`, "The frozen release is unchanged.", ...droppedWarning(result)].join("\n");
}

/**
 * Thrown when the Release commit can't be applied to main's tip: `paths` are the files both changed, and `advice` is
 * what to run, which depends on the action that met it.
 */
export class ReleaseConflict extends Error {
  constructor(paths, advice = "Cherry-pick the commit that changed them into the release (action: cherry-pick), or unfreeze.") {
    super(`main changed ${paths.map((path) => `\`${path}\``).join(", ")} after the frozen commit, and the release changes the same lines. ${advice} Nothing was changed.`);
    this.paths = paths;
  }
}

/** A git runner for the checkout at `cwd`: `git(args)` returns stdout trimmed, and throws with git's own message. */
export function gitIn(cwd) {
  return (args, options = {}) => execFileSync("git", args, { cwd, encoding: "utf8", maxBuffer: 1 << 28, stdio: ["ignore", "pipe", "pipe"], ...options }).trim();
}

const succeeds = (git, args) => {
  try {
    git(args);
    return true;
  } catch {
    return false;
  }
};
const revParse = (git, rev) => (succeeds(git, ["rev-parse", "--verify", "--quiet", `${rev}^{commit}`]) ? git(["rev-parse", "--verify", "--quiet", `${rev}^{commit}`]) : null);
const isAncestor = (git, ancestor, of) => succeeds(git, ["merge-base", "--is-ancestor", ancestor, of]);
/**
 * Whether `base` holds `commit`: as an ancestor, or cherry-picked by an earlier release, whose `-x` line names it. A
 * fix shipped on top of a release is on main and in that release's tag, which main's history does not reach.
 */
const holdsCommit = (git, commit, base) => isAncestor(git, commit, base) || git(["log", "-1", "--format=%H", "--fixed-strings", `--grep=(cherry picked from commit ${commit})`, base]) !== "";
const messageOf = (git, commit) => git(["log", "-1", "--format=%B", commit]);
const subjectOf = (git, commit) => git(["log", "-1", "--format=%s", commit]);

/** Fetches main, the release branches and the release tags from `remote`, replacing what an earlier fetch left. */
function fetchRelease(git, remote) {
  git(["fetch", "--quiet", "--prune", "--force", remote, `+refs/heads/main:${MAIN_REF}`, `+refs/heads/changeset-release/*:${REFS}/changeset-release/*`, "+refs/tags/v*:refs/tags/v*", "+refs/tags/@neralabs/*:refs/tags/@neralabs/*"]);
}

/** A frozen release's candidate, base and cherry-picks, or `broken`: why they can't be read. */
function readFrozen(git, candidate) {
  const tip = revParse(git, CANDIDATE_REF);
  const cut = tip === candidate ? readReleaseMessage(messageOf(git, tip)) : null;
  if (cut?.base !== undefined) return { candidate: tip, base: cut.base, picks: cut.picks, coreVersion: cut.coreVersion };
  const found = tip === null ? "is gone" : `is at ${short(tip)}`;
  return { broken: `the Release pull request names candidate ${short(candidate)}, but ${CANDIDATE_BRANCH} ${found}. Run the freeze action again, or unfreeze.` };
}

/**
 * The release as the remote holds it: main's tip, the Release pull request's head (null without one), and, frozen,
 * the candidate with its base and cherry-picks. `broken` says why a frozen release can't be read, for the actions that
 * need it; freezing again or unfreezing repairs it.
 */
export function readState(git) {
  const main = revParse(git, MAIN_REF);
  if (main === null) throw new Error("the remote has no main branch");
  const head = revParse(git, RELEASE_REF);
  const named = head === null ? null : readReleaseMessage(messageOf(git, head));
  // The version set by hand rides on the candidate's message: the head's own, while the release follows main.
  if (named?.candidate === undefined) return { main, head, frozen: false, coreVersion: named?.coreVersion ?? "" };
  return { main, head, frozen: true, coreVersion: "", ...readFrozen(git, named.candidate) };
}

/** The latest stable core release's version, among the `v*` tags the checkout holds, or null before the first. */
function latestReleaseVersion(git) {
  const versions = git(["tag", "--list", "v*"]).split("\n").map((tag) => tag.slice(1)).filter((version) => /^\d+\.\d+\.\d+$/u.test(version));
  return versions.sort((a, b) => compareVersions(b, a))[0] ?? null;
}

/** The commit of the latest stable core release's tag, or null before the first. */
export function latestReleaseCommit(git) {
  const latest = latestReleaseVersion(git);
  return latest === null ? null : revParse(git, `refs/tags/v${latest}`);
}

/** `picks` in the order main merged them: main's history is linear, so a later commit has more ancestors. */
const inMainOrder = (git, picks) => [...new Set(picks)].map((pick) => ({ pick, at: Number(git(["rev-list", "--count", pick])) })).sort((a, b) => a.at - b.at).map(({ pick }) => pick);

/**
 * Builds the candidate in the checkout: `base`, each of `picks` cherry-picked onto it in main's order, then the
 * Release commit `version()` writes, with core at `coreVersion` when one is set by hand. Returns the candidate's
 * commit, or null when nothing there is releasable.
 */
function buildCandidate({ git, version, coreVersion, setsVersion }, { base, picks, frozen }) {
  applyPicks(git, base, picks);
  const released = version(frozen ? { base, picks: picks.map((pick) => ({ commit: pick, subject: subjectOf(git, pick) })) } : null, coreVersion);
  // Only the run that sets a version is held to it: a later push to main rebuilds with what the release then is.
  if (setsVersion && released !== coreVersion) {
    git(["reset", "--quiet", "--hard"]);
    throw new Error(`core is not released as ${coreVersion}: it is not part of this release (nothing it ships changed since its last release, or the Release pull request's description says why it is left out), so its version can't be set. Nothing was changed.`);
  }
  git(["add", "--all"]);
  if (succeeds(git, ["diff", "--cached", "--quiet"])) return null;
  git(["commit", "--quiet", "--message", candidateMessage({ base, picks, coreVersion })]);
  return git(["rev-parse", "HEAD"]);
}

/** Leaves the checkout at `base` with each of `picks` cherry-picked onto it, in the order given. */
function applyPicks(git, base, picks) {
  git(["checkout", "--quiet", "--force", "--detach", base]);
  for (const pick of picks) {
    try {
      git(["cherry-pick", "-x", "--allow-empty", pick]);
    } catch {
      git(["cherry-pick", "--abort"]);
      throw new Error(
        `${short(pick)} ("${subjectOf(git, pick)}") does not apply to the frozen release: it builds on changes main made after ${short(base)}. ` +
          "Cherry-pick the commits it builds on first, or freeze again at a newer commit. Nothing was changed.",
      );
    }
  }
}

/** The tree of `base` with `picks` cherry-picked onto it: what a candidate's Release commit sits on. */
export function pickedTree(git, base, picks) {
  applyPicks(git, base, picks);
  return git(["rev-parse", "HEAD^{tree}"]);
}

/** The tree of the Release pull request's commit for `candidate` on `onto`: its Release commit applied there. */
export function pullRequestTree(git, candidate, onto) {
  return git(["rev-parse", `${buildPullRequestCommit({ git, candidate, onto }).commit}^{tree}`]);
}

/**
 * Applies `candidate`'s Release commit to `onto` (main's tip) for the Release pull request. A consumed entry main
 * changed since the freeze without it being cherry-picked is dropped, as the release ships it as frozen; any other
 * file both changed throws ReleaseConflict, leaving the checkout clean. Returns `{ commit, dropped }`.
 */
function buildPullRequestCommit({ git, candidate, onto, advice }) {
  git(["checkout", "--quiet", "--force", "--detach", onto]);
  let dropped = [];
  try {
    git(["cherry-pick", "--no-commit", candidate]);
  } catch {
    dropped = dropChangedEntries({ git, candidate, onto, advice });
  }
  git(["commit", "--quiet", "--message", pullRequestMessage(candidate)]);
  return { commit: git(["rev-parse", "HEAD"]), dropped };
}

/**
 * Settles a Release commit that did not apply cleanly: removes the consumed entries main changed, and returns them.
 * Any other file both changed throws ReleaseConflict, after putting the checkout back at `onto`.
 */
function dropChangedEntries({ git, candidate, onto, advice }) {
  const unmerged = git(["diff", "--name-only", "--diff-filter=U"]).split("\n").filter((path) => path !== "");
  const consumed = (path) => CONSUMED.some((folder) => path.startsWith(folder)) && !succeeds(git, ["cat-file", "-e", `${candidate}:${path}`]);
  const conflicts = unmerged.filter((path) => !consumed(path));
  if (unmerged.length > 0 && conflicts.length === 0) {
    for (const path of unmerged) git(["rm", "--quiet", "--force", "--", path]);
    return unmerged;
  }
  git(["reset", "--quiet", "--hard", onto]);
  if (unmerged.length === 0) throw new Error(`the Release commit ${short(candidate)} could not be applied to main. Nothing was changed.`);
  throw new ReleaseConflict(conflicts, advice);
}

/** Whether `head` squash-merges into `main` without a conflict. */
const mergesCleanly = (git, main, head) => succeeds(git, ["merge-tree", "--write-tree", main, head]);

/** `refs/heads/<branch>` on the remote must still be at `expected` (must not exist, for null) for the push to land. */
const lease = (branch, expected) => `--force-with-lease=refs/heads/${branch}:${expected ?? ""}`;

/** The commit `text` names (parseTarget), or null for none. */
function resolveCommit({ git, resolvePull }, text) {
  const named = parseTarget(text);
  if (named === null) return null;
  const rev = named.pull === undefined ? named.rev : resolvePull(named.pull);
  const commit = revParse(git, rev);
  if (commit === null) throw new Error(`${rev} is not a commit this checkout has: name a commit on main`);
  return commit;
}

/** The commit the action's target names, or null when none was given. */
const resolveTarget = (context) => resolveCommit(context, context.target);

/** The commits `fixes` names (commits on main or merged pull request numbers), each one `base` lacks. */
function resolveFixes(context, base) {
  const { git, state, fixes } = context;
  return fixes.split(/[\s,]+/u).filter((text) => text !== "").map((text) => {
    const commit = resolveCommit(context, text);
    const refusal = cherryPickRefusal({ commit, frozen: true, onMain: isAncestor(git, commit, state.main), included: holdsCommit(git, commit, base) });
    if (refusal !== null) throw new Error(refusal);
    return commit;
  });
}

/** Fails with why a frozen release can't be read. */
function assertReadable(state) {
  if (state.broken !== undefined) throw new Error(state.broken);
}

/** What an action returns for a frozen release: `changed` says whether what merging releases changed. */
const frozenResult = ({ candidate, base, picks }, changed, dropped) => ({ releasable: true, changed, frozen: true, candidate, base, picks, dropped });

/** Builds the frozen candidate and its pull request commit, and pushes both in one push. */
function freezeAt(context, base, picks) {
  const { git, state, push } = context;
  const ordered = inMainOrder(git, picks);
  const candidate = buildCandidate(context, { base, picks: ordered, frozen: true });
  if (candidate === null) throw new Error(`nothing is releasable at ${short(base)}: no pack has a changeset and nothing core ships changed since its last release. To ship a fix on top of a release, name the fix in the freeze action's fixes input. Nothing was changed.`);
  // Nothing is frozen yet when a first freeze meets a conflict, so there is nothing to cherry-pick onto.
  const advice = state.frozen ? undefined : "Name the commit that changed them in the freeze action's fixes input, or freeze a newer commit.";
  const { commit, dropped } = buildPullRequestCommit({ git, candidate, onto: state.main, advice });
  try {
    push(["--atomic", "--force", `${candidate}:refs/heads/${CANDIDATE_BRANCH}`, `${commit}:refs/heads/${RELEASE_BRANCH}`]);
  } catch (error) {
    throw workflowPushRefusal(error) ?? error;
  }
  return frozenResult({ candidate, base, picks: ordered }, true, dropped);
}

/**
 * GitHub refuses a push that adds a commit changing a workflow file unless the token may write workflows, which
 * release-pr.yml's does not ask for. The base's commits are on GitHub already, so only a cherry-pick can be refused so.
 */
function workflowPushRefusal(error) {
  if (!/workflows?` permission|without `?workflows?/iu.test(String(error.stderr ?? error.message))) return null;
  return new Error("GitHub refused the push: a cherry-picked commit changes a file under .github/workflows/, which this workflow's token may not push. Freeze a newer commit on main that holds that change instead. Nothing was changed.");
}

/** The cherry-picks a release frozen again at `base` keeps: those `base` does not hold, so moving it loses no fix. */
function keptPicks({ git, state }, base) {
  if (!state.frozen || state.broken !== undefined) return [];
  return state.picks.filter((pick) => !holdsCommit(git, pick, base));
}

function freeze(context) {
  const { git, state } = context;
  const base = resolveTarget(context) ?? state.main;
  const refusal = freezeRefusal({ commit: base, onMain: isAncestor(git, base, state.main), latestRelease: base === latestReleaseCommit(git), hasFlow: succeeds(git, ["cat-file", "-e", `${base}:${FLOW_MARKER}`]) });
  if (refusal !== null) throw new Error(refusal);
  return freezeAt(context, base, [...keptPicks(context, base), ...resolveFixes(context, base)]);
}

/** Whether the frozen release already holds `commit`, in its base or as a cherry-pick. */
const holds = ({ git, state }, commit) => state.frozen && (holdsCommit(git, commit, state.base) || state.picks.includes(commit));

function cherryPick(context) {
  const { git, state } = context;
  assertReadable(state);
  const commit = resolveTarget(context);
  if (commit === null) throw new Error("cherry-pick needs the commit on main, or the merged pull request's number, to add");
  const refusal = cherryPickRefusal({ commit, frozen: state.frozen, onMain: isAncestor(git, commit, state.main), included: holds(context, commit) });
  if (refusal !== null) throw new Error(refusal);
  return freezeAt(context, state.base, [...state.picks, commit]);
}

function unfreeze(context) {
  const { git, state, push } = context;
  if (!state.frozen) return { releasable: state.head !== null, changed: false, frozen: false, notice: "No release is frozen: the Release pull request already follows main." };
  const candidate = buildCandidate(context, { base: state.main, picks: [], frozen: false });
  const gone = revParse(git, CANDIDATE_REF) === null ? [] : [`:refs/heads/${CANDIDATE_BRANCH}`];
  // Nothing releasable on main: the Release pull request's branch goes too, and the workflow closes the pull request.
  push(["--atomic", "--force", ...gone, `${candidate ?? ""}:refs/heads/${RELEASE_BRANCH}`]);
  return { releasable: candidate !== null, changed: true, frozen: false };
}

/**
 * A push to main while frozen. The release is left alone, so the push neither reruns its checks nor clears its
 * checklist. Only when main has since changed a file the Release commit changes is that commit applied to main's tip
 * again, so the pull request stays mergeable; the candidate is the same, so the checklist stands.
 */
function updateFrozen({ git, state, push }) {
  assertReadable(state);
  if (mergesCleanly(git, state.main, state.head)) return frozenResult(state, false, []);
  let built;
  try {
    built = buildPullRequestCommit({ git, candidate: state.candidate, onto: state.main });
  } catch (error) {
    // Not the pusher's to fix, and the frozen release is whole: GitHub shows the pull request as conflicting, which
    // blocks the merge, and release-pr.yml says on it which files and what to run.
    if (!(error instanceof ReleaseConflict)) throw error;
    return { ...frozenResult(state, false, []), conflicts: error.paths };
  }
  push([lease(RELEASE_BRANCH, state.head), `${built.commit}:refs/heads/${RELEASE_BRANCH}`]);
  return frozenResult(state, false, built.dropped);
}

/** A push to main while the release follows main: the Release commit is rebuilt on main's tip. */
function updateFollowing(context) {
  const { state, push } = context;
  const candidate = buildCandidate(context, { base: state.main, picks: [], frozen: false });
  if (candidate === null) return { releasable: false, changed: false, frozen: false };
  try {
    push([lease(RELEASE_BRANCH, state.head), `${candidate}:refs/heads/${RELEASE_BRANCH}`]);
  } catch (error) {
    // The branch moved since it was read: an action run by hand or a newer push's run landed meanwhile, and it wins.
    if (!/stale info|rejected/u.test(String(error.stderr ?? error.message))) throw error;
    return { releasable: true, changed: false, frozen: false, notice: "The Release pull request changed while this run was building: left as it is." };
  }
  return { releasable: true, changed: true, frozen: false };
}

/**
 * Sets the version core releases as, or, for none, goes back to the one release-version.mjs picks. The release is
 * rebuilt as it is, frozen or following main, with that version, which later rebuilds keep until the release merges.
 */
function setVersion(context) {
  const { state } = context;
  assertReadable(state);
  return state.frozen ? freezeAt(context, state.base, state.picks) : updateFollowing(context);
}

const ACTIONS = { freeze, "cherry-pick": cherryPick, unfreeze, "set-version": setVersion, update: (context) => (context.state.frozen ? updateFrozen(context) : updateFollowing(context)) };

/**
 * The version core releases as in this run, and whether this run sets it: the one asked for, by the set-version action
 * (none goes back to release-version.mjs's choice) or beside any other action, else the one set earlier, which the
 * release's message carries. One set now must be a plain version later than the latest release.
 */
function coreVersionOf({ action, coreVersion, git, state }) {
  const sets = action === "set-version" || coreVersion !== "";
  if (!sets) return { coreVersion: state.coreVersion ?? "", setsVersion: false };
  const refusal = coreVersion === "" ? null : coreVersionRefusal(coreVersion, latestReleaseVersion(git));
  if (refusal !== null) throw new Error(`${refusal} Nothing was changed.`);
  return { coreVersion, setsVersion: coreVersion !== "" };
}

/**
 * Runs one action of release-pr.yml against `remote` from the clean checkout `git` runs in, and returns what the
 * workflow does next:
 *   - `releasable`: false when nothing is releasable, so the Release pull request is closed;
 *   - `changed`: whether what merging releases changed, so the description is rewritten and its checklist cleared;
 *   - `frozen`, and frozen: `candidate`, `base`, `picks`;
 *   - `dropped`: consumed entries main changed after the freeze, released as frozen;
 *   - `notice`: a line for the run summary when the action had nothing to do.
 * Every action changes the remote in one push or not at all, and an action that can't run throws saying what to run.
 *
 * `action` is `update` (a push to main), `freeze`, `cherry-pick`, `unfreeze` or `set-version`; `coreVersion` is the
 * version core releases as, set by hand (coreVersionOf), and the result carries the one in force. `target` is a commit, release tag or
 * pull request number (parseTarget): the commit to freeze (main's tip when empty) or to cherry-pick. `fixes`, for
 * freeze alone, lists commits to cherry-pick in the same step, which is how a fix ships on top of the latest release:
 * that release alone has nothing to release. `version(frozen)` writes the Release commit's changes into the checkout
 * (release-version.mjs), given `{ base, picks: [{ commit, subject }] }` for a frozen candidate and null otherwise, then
 * the version core releases as when one is set, and returns core's version in the checkout once it has run;
 * `resolvePull(number)` gives a merged pull request's commit on main.
 */
export function runReleaseAction({ action, target = "", fixes = "", coreVersion = "", git, remote, version, resolvePull }) {
  const run = Object.hasOwn(ACTIONS, action) ? ACTIONS[action] : undefined;
  if (run === undefined) throw new Error(`unknown action "${action}": freeze, cherry-pick, unfreeze or set-version`);
  if (git(["status", "--porcelain"]) !== "") throw new Error("the checkout has changes: run this from a clean checkout, as release-pr.yml does");
  fetchRelease(git, remote);
  const state = readState(git);
  const core = coreVersionOf({ action, coreVersion, git, state });
  const result = run({ git, target, fixes, version, resolvePull, state, ...core, push: (args) => git(["push", "--quiet", remote, ...args]) });
  return core.coreVersion === "" ? result : { ...result, coreVersion: core.coreVersion };
}
