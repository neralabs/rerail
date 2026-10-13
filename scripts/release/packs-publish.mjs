#!/usr/bin/env node
// Publishes the tarballs packs-pack.ts wrote to <dir>, each under `next` and restricted, checks that the registry
// does not show it to an anonymous reader, and tags the released commit `<name>@<version>` on GitHub. Stops at the
// first failure: packs published before it keep their tags, so a re-run publishes only the rest.
//
// In the release workflow npm authenticates through trusted publishing and the commit is $GITHUB_SHA. By hand (a
// pack's first publish), npm uses your login and the commit is HEAD, which must be clean and pushed.
//
//   node scripts/release/packs-publish.mjs <dir>
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { createReleaseTag, PUBLISH_TAG, REGISTRY, releaseTag, ROOT } from "./packs.mjs";

const git = (args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8" }).trim();

/** The commit the tags name: the workflow's, or a clean HEAD. */
function releasedCommit() {
  if (process.env.GITHUB_SHA !== undefined) return process.env.GITHUB_SHA;
  if (git(["status", "--porcelain"]) !== "") throw new Error("the checkout has changes; publish from a clean checkout of the commit to tag");
  return git(["rev-parse", "HEAD"]);
}

/** Fails unless an anonymous request for `name` is refused, as it is for a restricted package. */
async function assertRestricted(name) {
  const response = await fetch(`${REGISTRY}/${name.replace("/", "%2f")}`, { headers: { accept: "application/json" } });
  if (response.status !== 404) throw new Error(`${name} answers an anonymous request with ${response.status}: it is not restricted. Run npm access set status=private ${name}`);
}

const [directory] = process.argv.slice(2);
if (directory === undefined) throw new Error("usage: packs-publish.mjs <dir written by packs-pack.ts>");
const out = resolve(directory);
const manifest = JSON.parse(readFileSync(join(out, "manifest.json"), "utf8"));
const commit = releasedCommit();

for (const { name, version, file } of manifest) {
  execFileSync("npm", ["publish", join(out, file), "--registry", REGISTRY, "--access", "restricted", "--tag", PUBLISH_TAG], { stdio: "inherit" });
  await assertRestricted(name);
  const tag = releaseTag(name, version);
  createReleaseTag(tag, commit);
  console.log(`published ${tag} under ${PUBLISH_TAG}, tagged ${commit}`);
}
