#!/usr/bin/env node
// Refuses release.yml's `publish` input unless it retries a release with main's core code the same as the tag's
// (publishRetryRefusal in core-paths.mjs), and lists what did change in the run summary for whoever approves the publish
// (retrySummary). It runs before `npm ci`, so it uses nothing but core-paths.mjs.
//
//   VERSION=0.1.2 [LATEST=<version>] GITHUB_REF=refs/heads/main GITHUB_REPOSITORY=owner/name node scripts/release/core-retry-check.mjs
import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { coreChanges, publishRetryRefusal, retrySummary } from "./core-paths.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const git = (args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8" }).trim();

const { VERSION: version = "", LATEST: latest = "", GITHUB_REF: ref = "", GITHUB_REPOSITORY: repository = "" } = process.env;
const packageVersion = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")).version;

/**
 * Fetches the version's tag, which a shallow checkout lacks: whether it exists. Only a valid version reaches this. Only
 * `git ls-remote` finding no such ref (its exit code 2) means no tag: a network or auth failure stops the run with git's
 * own message, since re-running may be all it needs. A release needs a public repository for its provenance, so no token.
 * `--depth` only where the checkout is shallow already: in a full clone it would cut the clone's history at the tag.
 */
function fetchTag() {
  const url = `https://github.com/${repository}.git`;
  const tag = `refs/tags/v${version}`;
  try {
    git(["ls-remote", "--exit-code", url, tag]);
  } catch (error) {
    if (error.status === 2) return false;
    throw error;
  }
  const depth = git(["rev-parse", "--is-shallow-repository"]) === "true" ? ["--depth=1"] : [];
  git(["fetch", "--quiet", "--force", ...depth, url, `${tag}:${tag}`]);
  return true;
}

const tagName = `v${version}`;
const changedSinceTag = () => git(["diff", "--name-only", "--no-renames", tagName, "HEAD"]).split("\n").filter((path) => path !== "");
const readLock = (at) => git(["show", `${at === "release" ? tagName : "HEAD"}:package-lock.json`]);

const before = publishRetryRefusal({ version, ref, latest, packageVersion, tagged: true, shipped: [] });
const tagged = before === null && fetchTag();
const changed = tagged ? changedSinceTag() : [];
const refusal = before ?? publishRetryRefusal({ version, ref, latest, packageVersion, tagged, shipped: coreChanges(changed, readLock) });
if (refusal !== null) {
  console.error(`::error::${refusal}`);
  process.exit(1);
}
const summary = retrySummary(version, changed);
console.log(summary);
if (process.env.GITHUB_STEP_SUMMARY !== undefined) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary);
