#!/usr/bin/env node
// Releases core when main's package.json names a version with no `v<version>` tag: tags, on GitHub, the commit that
// set that version (the merged Release pull request), and the tag's push starts release.yml, which builds and
// publishes it. A version that is not later than every released one is refused (coreTagRefusal in core.mjs).
//
// The tag must be created with a token that starts workflows: in release-pr.yml, the neralabs-ci app's (a tag
// GITHUB_TOKEN creates starts none). Needs the full history, to find the commit, and gh with write access.
//
//   node scripts/release/core-tag.mjs   (GH_REPO or the checkout's remote names the repository)
import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { coreTag, coreTagRefusal } from "./core.mjs";
import { createReleaseTag, ROOT } from "./packs.mjs";

const run = (command, args) => execFileSync(command, args, { cwd: ROOT, encoding: "utf8" }).trim();

const version = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")).version;
const tag = coreTag(version);
const tags = run("gh", ["api", "--paginate", "repos/{owner}/{repo}/git/matching-refs/tags/v", "--jq", ".[].ref"])
  .split("\n")
  .filter((ref) => ref !== "")
  .map((ref) => ref.replace(/^refs\/tags\//u, ""));

if (tags.includes(tag)) {
  console.log(`core ${version} is released (${tag} exists)`);
} else {
  const refusal = coreTagRefusal(version, tags);
  if (refusal !== null) throw new Error(`not tagging core: ${refusal}`);
  // The newest commit that added or removed this version string in package.json: the one that set it.
  const commit = run("git", ["log", "-1", "--format=%H", `-S"version": "${version}"`, "--", "package.json"]);
  if (commit === "" || JSON.parse(run("git", ["show", `${commit}:package.json`])).version !== version) {
    throw new Error(`no commit on this branch sets package.json's version to ${version}`);
  }
  createReleaseTag(tag, commit);
  console.log(`tagged ${tag} at ${commit}: release.yml builds and publishes it`);
  if (process.env.GITHUB_STEP_SUMMARY !== undefined) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `Tagged \`${tag}\` at ${commit}: the Release workflow on that tag builds and publishes core ${version}.\n`);
  }
}
