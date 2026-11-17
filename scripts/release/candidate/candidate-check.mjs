#!/usr/bin/env node
// The Release pull request's merge guard, in ci.yml's release-candidate job, which "CI result" needs. Merging tags the
// commit the pull request's head names (release-tag.mjs), so before the merge:
//   - the head must be the commit release-pr.yml built: not one pushed by hand or made by GitHub's "Update branch",
//     which would merge everything on main into the release (pullRequestHeadRefusal in candidate.mjs);
//   - frozen, the candidate it names must be the candidate branch's tip, and CI must have passed on that exact commit.
//     release-pr.yml starts that CI run when it freezes or cherry-picks; this waits for it, up to $WAIT_MINUTES (100
//     when unset), so the check goes green by itself. Following main, the release is main's tip, which main's own CI
//     covers, as before.
// Needs the full history and gh.
//
//   HEAD_SHA=<the pull request's head> node scripts/release/candidate/candidate-check.mjs   (GH_REPO names the repository)
import { execFileSync } from "node:child_process";
import { setTimeout as sleep } from "node:timers/promises";
import { CANDIDATE_BRANCH, candidateCiVerdict, gitIn, pullRequestHeadRefusal, readReleaseMessage, short } from "./candidate.mjs";
import { ROOT } from "../packs.mjs";

const { HEAD_SHA: head = "", WAIT_MINUTES: waitMinutes = "100" } = process.env;
if (!/^[0-9a-f]{40}$/u.test(head)) throw new Error("HEAD_SHA must be the pull request's head commit");

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
const versionAt = (commit) => JSON.parse(git(["show", `${commit}:package.json`])).version;
const messageAt = (commit) => readReleaseMessage(git(["log", "-1", "--format=%B", commit]));

git(["fetch", "--quiet", "--prune", "--force", "origin", "+refs/heads/main:refs/release/main", "+refs/heads/changeset-release/*:refs/release/changeset-release/*"]);

/** What pullRequestHeadRefusal reads of the candidate a frozen head names. */
function candidateFacts(named) {
  const tip = tryGit(["rev-parse", "--verify", "--quiet", `refs/release/${CANDIDATE_BRANCH}^{commit}`]);
  if (tip !== named.candidate) return { tip };
  const cut = messageAt(tip);
  const commits = cut?.base === undefined ? 0 : Number(git(["rev-list", "--count", `${cut.base}..${tip}`]));
  return { tip, named: cut, commits, version: versionAt(tip) };
}

const named = messageAt(head);
const parents = git(["rev-list", "--parents", "-n", "1", head]).split(" ").slice(1);
const parentOnMain = parents.length === 1 && tryGit(["merge-base", "--is-ancestor", parents[0], "refs/release/main"]) !== null;
const refusal = pullRequestHeadRefusal({
  head: { named, parents, parentOnMain, version: versionAt(head) },
  candidate: named?.candidate === undefined ? undefined : candidateFacts(named),
});
if (refusal !== null) fail(refusal);

if (named.candidate === undefined) {
  console.log(`The release follows main: merging releases ${short(head)}, main's tip ${short(named.base)} and the Release commit.`);
  process.exit(0);
}

/** The "CI result" check runs on the candidate. */
const checkRuns = () => JSON.parse(execFileSync("gh", ["api", `repos/{owner}/{repo}/commits/${named.candidate}/check-runs?check_name=CI%20result&per_page=100`, "--jq", ".check_runs"], { cwd: ROOT, encoding: "utf8" }));
const rerun = "run `gh workflow run ci.yml --ref changeset-release/candidate`, then re-run this job";
const deadline = Date.now() + Number(waitMinutes) * 60_000;
for (;;) {
  const verdict = candidateCiVerdict(checkRuns());
  if (verdict === "passed") break;
  if (verdict === "failed") fail(`CI failed on candidate ${short(named.candidate)}. Fix it on main and cherry-pick the fix (the candidate's CI then runs again), or, for a flaky run, ${rerun}.`);
  if (Date.now() > deadline) fail(`CI has not finished on candidate ${short(named.candidate)} after ${waitMinutes} minutes. Re-run this job once it has; if no CI run exists on the candidate, ${rerun}.`);
  console.log(`Waiting for CI on candidate ${short(named.candidate)}...`);
  await sleep(60_000);
}
console.log(`CI passed on candidate ${short(named.candidate)}: merging releases exactly that commit.`);
