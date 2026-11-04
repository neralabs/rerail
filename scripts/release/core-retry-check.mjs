#!/usr/bin/env node
// Refuses release.yml's `publish` input unless it retries a release that never published what is on main now
// (publishRetryRefusal in core-paths.mjs). It runs before `npm ci`, so it uses nothing but core-paths.mjs.
//
//   VERSION=0.1.2 GITHUB_REF=refs/heads/main GITHUB_REPOSITORY=owner/name GH_TOKEN=... node scripts/release/core-retry-check.mjs
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { coreChanges, publishRetryRefusal } from "./core-paths.mjs";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const git = (args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8" }).trim();

const { VERSION: version = "", GITHUB_REF: ref = "", GITHUB_REPOSITORY: repository = "", GH_TOKEN: token = "" } = process.env;
const packageVersion = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")).version;

/**
 * Fetches the version's tag, which a shallow checkout lacks: whether it exists. Only a valid version reaches this.
 * `--depth` only where the checkout is shallow already: in a full clone it would cut the clone's history at the tag.
 */
function fetchTag() {
  const tag = `refs/tags/v${version}`;
  const depth = git(["rev-parse", "--is-shallow-repository"]) === "true" ? ["--depth=1"] : [];
  try {
    git(["fetch", "--quiet", "--force", ...depth, `https://x-access-token:${token}@github.com/${repository}.git`, `${tag}:${tag}`]);
    return true;
  } catch {
    return false;
  }
}

/** The shipped paths changed since the version's tag. */
function shippedSinceTag() {
  const tag = `v${version}`;
  const changed = git(["diff", "--name-only", "--no-renames", tag, "HEAD"]).split("\n").filter((path) => path !== "");
  return coreChanges(changed, (at) => git(["show", `${at === "release" ? tag : "HEAD"}:package-lock.json`]));
}

const before = publishRetryRefusal({ version, ref, packageVersion, tagged: true, shipped: [] });
const tagged = before === null && fetchTag();
const refusal = before ?? publishRetryRefusal({ version, ref, packageVersion, tagged, shipped: tagged ? shippedSinceTag() : [] });
if (refusal !== null) {
  console.error(`::error::${refusal}`);
  process.exit(1);
}
console.log(`Retrying ${version}: nothing core ships has changed since v${version}.`);
