#!/usr/bin/env node
// On a `release-*` tag's push, before release-packs.yml plans, tags or publishes anything: the tag must be the one
// release-pr.yml makes when the Release pull request merges, named for its commit, at a commit a merged Release pull
// request released (releaseTrainRefusal in candidate.mjs). A tag pushed by hand at another commit releases nothing.
// It stops a mistake, not an attacker: a tag's run uses the tagged commit's own workflow, so who may create these
// tags is the tag ruleset's to limit (docs/release.md, "Freezing a release"). It runs before `npm ci`.
//
//   GITHUB_REF_NAME=release-<commit> GITHUB_SHA=<commit> node scripts/release/candidate/release-train-check.mjs   (GH_REPO names the repository)
import { releaseTrainRefusal, short } from "./candidate.mjs";
import { mergedReleaseHeads, releasedBy } from "./merged.mjs";

const { GITHUB_REF_NAME: tag = "", GITHUB_SHA: commit = "" } = process.env;

/** The commits merged Release pull requests released, last merged first, up to the one that released `commit`. */
function releasedCommits() {
  const released = [];
  for (const { head } of mergedReleaseHeads()) {
    const found = releasedBy(head);
    if (found !== null) released.push(found);
    if (found === commit) break;
  }
  return released;
}

const refusal = releaseTrainRefusal({ tag, commit, released: releasedCommits() });
if (refusal !== null) {
  console.error(`::error title=Not a release::${refusal}`);
  process.exit(1);
}
console.log(`${tag} is the release a merged Release pull request made at ${short(commit)}.`);
