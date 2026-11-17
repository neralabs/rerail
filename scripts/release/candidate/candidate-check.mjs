#!/usr/bin/env node
// The Release pull request's merge guard, in ci.yml's release-candidate job, which "CI result" needs. Merging tags the
// commit the pull request's head names (release-tag.mjs), so before the merge:
//   - the head must be the commit release-pr.yml built: not one pushed by hand or made by GitHub's "Update branch",
//     which would merge everything on main into the release (pullRequestHeadRefusal in candidate.mjs);
//   - frozen, the candidate it names is not in the pull request's diff, which shows only the Release commit. So the
//     candidate is rebuilt here and must come out the same: its base on main or the latest release, each cherry-pick
//     a commit on main, nothing else in it, and a Release commit that does only what the pull request shows
//     (candidateTrustRefusal). What merges is then reviewed commits and the changes a reviewer sees;
//   - frozen, ci.yml must have passed on that exact commit. release-pr.yml starts that run when it freezes or
//     cherry-picks; this waits for it, up to $WAIT_MINUTES (100 when unset), so the check goes green by itself.
//     Following main, the release is main's tip, which main's own CI covers, as before.
// Needs the full history and gh, and leaves the checkout where it found it.
//
//   HEAD_SHA=<the pull request's head> node scripts/release/candidate/candidate-check.mjs   (GH_REPO names the repository)
import { execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { CANDIDATE_BRANCH, candidateCiVerdict, candidateTrustRefusal, gitIn, latestReleaseCommit, pickedTree, pullRequestHeadRefusal, pullRequestTree, readReleaseMessage, short } from "./candidate.mjs";
import { ROOT } from "../packs.mjs";

const { HEAD_SHA: head = "", WAIT_MINUTES: waitMinutes = "100" } = process.env;
if (!/^[0-9a-f]{40}$/u.test(head)) throw new Error("HEAD_SHA must be the pull request's head commit");
// The rebuild commits in the checkout; nothing it makes is pushed.
for (const name of ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME"]) process.env[name] ??= "release-candidate-check";
for (const name of ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL"]) process.env[name] ??= "release-candidate-check@users.noreply.github.com";

const MAIN = "refs/release/main";
const git = gitIn(ROOT);
const tryGit = (args) => {
  try {
    return git(args);
  } catch {
    return null;
  }
};
const fail = (message) => {
  console.error(`::error title=Release candidate::${message}`);
  process.exit(1);
};
const onMain = (commit) => tryGit(["merge-base", "--is-ancestor", commit, MAIN]) !== null;
const treeOf = (commit) => git(["rev-parse", `${commit}^{tree}`]);
const versionAt = (commit) => JSON.parse(git(["show", `${commit}:package.json`])).version;
const messageAt = (commit) => readReleaseMessage(git(["log", "-1", "--format=%B", commit]));
/** Whether `build()` gives the tree `expected`: a rebuild that fails, such as a cherry-pick that does not apply, does not. */
const rebuilds = (build, expected) => {
  try {
    return build() === expected;
  } catch {
    return false;
  }
};

git(["fetch", "--quiet", "--prune", "--force", "origin", `+refs/heads/main:${MAIN}`, "+refs/heads/changeset-release/*:refs/release/changeset-release/*", "+refs/tags/v*:refs/tags/v*"]);

/** What pullRequestHeadRefusal reads of the candidate a frozen head names. */
function candidateFacts(named) {
  const tip = tryGit(["rev-parse", "--verify", "--quiet", `refs/release/${CANDIDATE_BRANCH}^{commit}`]);
  if (tip !== named.candidate) return { tip };
  const cut = messageAt(tip);
  const commits = cut?.base === undefined ? 0 : Number(git(["rev-list", "--count", `${cut.base}..${tip}`]));
  return { tip, named: cut, commits, version: versionAt(tip) };
}

/** What candidateTrustRefusal reads: the candidate rebuilt from its base and cherry-picks, and its Release commit applied where the head sits. */
function trustFacts(candidate, { base, picks }, parent) {
  const facts = {
    candidate,
    baseTrusted: onMain(base) || base === latestReleaseCommit(git),
    picksOnMain: picks.every(onMain),
    picksMatch: rebuilds(() => pickedTree(git, base, picks), treeOf(`${candidate}^`)),
    releaseMatches: rebuilds(() => pullRequestTree(git, candidate, parent), treeOf(head)),
  };
  git(["checkout", "--quiet", "--force", "--detach", head]);
  return facts;
}

const named = messageAt(head);
const parents = git(["rev-list", "--parents", "-n", "1", head]).split(" ").slice(1);
const frozen = named?.candidate === undefined ? undefined : candidateFacts(named);
const refusal = pullRequestHeadRefusal({ head: { named, parents, parentOnMain: parents.length === 1 && onMain(parents[0]), version: versionAt(head) }, candidate: frozen });
if (refusal !== null) fail(refusal);

if (frozen === undefined) {
  console.log(`The release follows main: merging releases ${short(head)}, main's tip ${short(named.base)} and the Release commit.`);
  process.exit(0);
}

const untrusted = candidateTrustRefusal(trustFacts(named.candidate, frozen.named, parents[0]));
if (untrusted !== null) fail(untrusted);
console.log(`Candidate ${short(named.candidate)} is its base, ${frozen.named.picks.length} cherry-pick(s) from main, and the Release commit this pull request shows.`);

/** ci.yml's runs on the candidate that release-pr.yml or a maintainer started: a pull request's run skips jobs. */
const ciRuns = () => JSON.parse(execFileSync("gh", ["api", `repos/{owner}/{repo}/actions/workflows/ci.yml/runs?head_sha=${named.candidate}&event=workflow_dispatch&per_page=100`, "--jq", ".workflow_runs"], { cwd: ROOT, encoding: "utf8" }));
const rerun = "run `gh workflow run ci.yml --ref changeset-release/candidate`, then re-run this job";
const deadline = Date.now() + Number(waitMinutes) * 60_000;
for (;;) {
  const verdict = candidateCiVerdict(ciRuns());
  if (verdict === "passed") break;
  if (verdict === "failed") fail(`CI failed on candidate ${short(named.candidate)}. Fix it on main and cherry-pick the fix (the candidate's CI then runs again), or, for a flaky run, ${rerun}.`);
  if (Date.now() > deadline) fail(`CI has not finished on candidate ${short(named.candidate)} after ${waitMinutes} minutes. Re-run this job once it has; if no CI run exists on the candidate, ${rerun}.`);
  console.log(`Waiting for CI on candidate ${short(named.candidate)}...`);
  await sleep(60_000);
}
console.log(`CI passed on candidate ${short(named.candidate)}: merging releases exactly that commit.`);
