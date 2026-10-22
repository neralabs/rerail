#!/usr/bin/env node
// Publishes the tarballs packs-pack.ts wrote to <dir>, each restricted and under `latest` (under `next` for a pack
// named in $HELD, space-separated ids that packs-plan.mjs found held at an older `latest`), checks that the registry
// does not show it to an anonymous reader, and tags the released commit `<name>@<version>` on GitHub. Any failed run
// can be run again, with the same tarballs or fresh ones: a version already tagged is skipped, and one already on the
// registry without its tag is checked and tagged, not published again (releasePacks in packs.mjs).
//
// In the release workflow npm authenticates through trusted publishing and the commit is $GITHUB_SHA. By hand (a
// pack's first publish), npm uses your login, gh your GitHub login with write access, and the commit is HEAD, which
// must be clean and pushed. Both are checked before anything is published. Writes `released` (each
// `<name>@<version>:<dist-tag>` it released, space-separated, for dist-tags-check.mjs) to $GITHUB_OUTPUT when it is set.
//
//   [HELD="<id> ..."] node scripts/release/packs-publish.mjs <dir>
import { execFileSync, spawnSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { assertCommitOnGitHub, createReleaseTag, listPacks, REGISTRY, releasePacks, releaseTags, ROOT } from "./packs.mjs";

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

/**
 * Runs npm publish for one tarball; whether it succeeded, and its error output. stdin and stdout stay the terminal:
 * npm asks for a 2FA code or browser approval only when both are one. Its `npm error` lines, the publish conflict
 * included, go to stderr, which is captured and shown when npm exits.
 */
function publish(path, distTag) {
  const run = spawnSync("npm", ["publish", path, "--registry", REGISTRY, "--access", "restricted", "--tag", distTag], { stdio: ["inherit", "inherit", "pipe"], encoding: "utf8" });
  process.stderr.write(run.stderr ?? "");
  if (run.error !== undefined) throw run.error;
  return { ok: run.status === 0, output: run.stderr };
}

const [directory] = process.argv.slice(2);
if (directory === undefined) throw new Error("usage: packs-publish.mjs <dir written by packs-pack.ts>");
const out = resolve(directory);
const manifest = JSON.parse(readFileSync(join(out, "manifest.json"), "utf8"));
const commit = releasedCommit();
assertCommitOnGitHub(commit);

const heldIds = (process.env.HELD ?? "").split(/\s+/u).filter((id) => id !== "");
const held = listPacks().filter((pack) => heldIds.includes(pack.id)).map((pack) => pack.name);

const released = await releasePacks(
  manifest,
  releaseTags(),
  {
    publish: (file, distTag) => publish(join(out, file), distTag),
    assertRestricted,
    tag: (tag) => createReleaseTag(tag, commit),
    log: (line) => console.log(line),
  },
  held,
);
if (process.env.GITHUB_OUTPUT !== undefined) {
  appendFileSync(process.env.GITHUB_OUTPUT, `released=${released.map(({ name, version, distTag }) => `${name}@${version}:${distTag}`).join(" ")}\n`);
}
