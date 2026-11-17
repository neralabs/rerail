#!/usr/bin/env node
// Tags what merging the Release pull request released (docs/release.md, "Freezing a release"): the commit the merged
// pull request's head names (releasedCommit in candidate.mjs), which is the frozen candidate, or the head itself when
// the release followed main. Never main's merge commit: frozen, that holds everything merged since the freeze.
//   - `v<version>`, when the released commit's package.json names a core version with no tag yet: its push starts
//     release.yml, which builds and publishes core. A version that is not later than every released one is refused
//     (coreTagRefusal in core.mjs), and the packs still release. So do they when core's tag can't be made: every later
//     run tries it again until it is, and fails saying why until then.
//   - `release-<commit>` (releaseTrainTag): its push starts release-packs.yml, which publishes every pack whose version
//     at that commit has no tag. One per release, so the packs take one run and one approval.
// Then it removes the candidate's branch, which the tags have replaced.
//
// It reads the latest merged Release pull request, not the push that started it, so a run cancelled by a newer push to
// main loses nothing: the next run tags the same commit. A release already tagged is left alone. The tags must be
// created with a token that starts workflows: in release-pr.yml, the neralabs-ci app's (a tag GITHUB_TOKEN creates
// starts none).
//
//   node scripts/release/release-tag.mjs   (GH_REPO or the checkout's remote names the repository)
import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { CANDIDATE_BRANCH, releaseTrainTag, short } from "./candidate/candidate.mjs";
import { mergedReleaseHeads, releasedBy } from "./candidate/merged.mjs";
import { coreTag, coreTagRefusal } from "./core.mjs";
import { createReleaseTag, ROOT } from "./packs.mjs";

const gh = (args) => execFileSync("gh", args, { cwd: ROOT, encoding: "utf8" }).trim();
const api = (path, args = []) => gh(["api", path, ...args]);
const tagsUnder = (prefix) => api(`repos/{owner}/{repo}/git/matching-refs/tags/${prefix}`, ["--paginate", "--jq", ".[].ref"]).split("\n").filter((ref) => ref !== "").map((ref) => ref.replace(/^refs\/tags\//u, ""));
const say = (line) => {
  console.log(line);
  if (process.env.GITHUB_STEP_SUMMARY !== undefined) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${line}\n\n`);
};

/**
 * Tags core's version at `released`, unless it is tagged already; returns why the version is refused, or null. A run
 * after the packs' tag (`again`) says nothing in the run summary unless it tags.
 */
function tagCore(released, again) {
  const version = JSON.parse(api(`repos/{owner}/{repo}/contents/package.json?ref=${released}`, ["-H", "Accept: application/vnd.github.raw"])).version;
  const tags = tagsUnder("v");
  if (tags.includes(coreTag(version))) {
    if (!again) say(`Core ${version} is released already (\`${coreTag(version)}\` exists), so this release is the packs' alone.`);
    return null;
  }
  const refusal = coreTagRefusal(version, tags);
  if (refusal !== null) return `not tagging core: ${refusal}`;
  createReleaseTag(coreTag(version), released);
  say(`Tagged \`${coreTag(version)}\` at ${released}: the Release workflow on that tag builds and publishes core ${version}.`);
  return null;
}

/** Removes the candidate's branch when it is still at the released commit: the tags hold the commit now. */
function removeCandidateBranch(released) {
  let tip = "";
  try {
    tip = api(`repos/{owner}/{repo}/git/ref/heads/${CANDIDATE_BRANCH}`, ["--jq", ".object.sha"]);
  } catch {
    return;
  }
  if (tip === released) api(`repos/{owner}/{repo}/git/refs/heads/${CANDIDATE_BRANCH}`, ["-X", "DELETE", "--silent"]);
}

function main() {
  const [merged] = mergedReleaseHeads();
  if (merged === undefined) return say("No Release pull request has merged: nothing to tag.");
  const { head } = merged;
  const released = releasedBy(head);
  // Every push to main reaches the two lines below until the next release, so they stay out of the run summary.
  if (released === null) return console.log(`Release pull request ${merged.number}'s head ${short(head)} was not built by this workflow (it is from before it, or was changed by hand): nothing is tagged.`);
  const train = releaseTrainTag(released);
  const again = tagsUnder(train).includes(train);
  // Core's tag apart from the packs', so one that can't be made never holds the packs back.
  let failure = null;
  try {
    const refusal = tagCore(released, again);
    // A refused version stays refused: said once, by the run that tags the packs.
    if (!again) failure = refusal;
  } catch (error) {
    failure = `tagging core failed, so its tag may not have been made; while it is missing, the next push to main makes it: ${error.message}`;
  }
  if (again) console.log(`Release pull request ${merged.number} is released: ${train} exists.`);
  else {
    createReleaseTag(train, released);
    say(`Tagged \`${train}\` at ${released} (Release pull request ${merged.number}): the Release packs workflow on that tag publishes the packs.`);
  }
  // A run cancelled after tagging left the branch behind.
  removeCandidateBranch(released);
  if (failure !== null) throw new Error(failure);
  return undefined;
}

main();
