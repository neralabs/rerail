#!/usr/bin/env node
// Tags the released commit `<name>@<version>` on GitHub for each pack in the manifest packs-pack.ts wrote to <dir>,
// before packs-publish.mjs publishes it, so a pack's release never waits on npm. A version already tagged is skipped,
// so a failed run can be run again. Writes `released` (each `<name>@<version>:<dist-tag>` in the manifest,
// space-separated, `next` for a pack named in $HELD, for github-release.mjs) to $GITHUB_OUTPUT when it is set.
//
// In the release workflow the commit is $GITHUB_SHA; by hand (a pack's first publish), HEAD, which must be clean and
// pushed, and gh uses your GitHub login with write access. Both are checked before anything is tagged.
//
//   [HELD="<id> ..."] node scripts/release/packs-tag.mjs <dir>
import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { assertCommitOnGitHub, createReleaseTag, heldPackNames, releaseTags, ROOT, tagPacks } from "./packs.mjs";

const git = (args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8" }).trim();

/** The commit the tags name: the workflow's, or a clean HEAD. */
function releasedCommit() {
  if (process.env.GITHUB_SHA !== undefined) return process.env.GITHUB_SHA;
  if (git(["status", "--porcelain"]) !== "") throw new Error("the checkout has changes; tag from a clean checkout of the commit to release");
  return git(["rev-parse", "HEAD"]);
}

const [directory] = process.argv.slice(2);
if (directory === undefined) throw new Error("usage: packs-tag.mjs <dir written by packs-pack.ts>");
const manifest = JSON.parse(readFileSync(join(resolve(directory), "manifest.json"), "utf8"));
const commit = releasedCommit();
assertCommitOnGitHub(commit);

const released = tagPacks(
  manifest,
  releaseTags(),
  { tag: (tag) => createReleaseTag(tag, commit), log: (line) => console.log(line) },
  heldPackNames(process.env.HELD),
);
if (process.env.GITHUB_OUTPUT !== undefined) {
  appendFileSync(process.env.GITHUB_OUTPUT, `released=${released.map(({ name, version, distTag }) => `${name}@${version}:${distTag}`).join(" ")}\n`);
}
