#!/usr/bin/env node
// Publishes the tarballs packs-pack.ts wrote to <dir>, tagged already by packs-tag.mjs, each public and under
// `latest` (under `next` for a pack named in $HELD, space-separated ids that packs-plan.mjs found held at an older
// `latest`), and points `next` at a version published under `latest` unless `next` is already at a newer one (so
// `next` never trails `latest`). It waits on nothing npm does after a publish: the release workflow's dist-tag check
// reads the result back. Any failed run can be run again, with the same tarballs or fresh ones: a version already on
// the registry has its `next` raised, not published again (releasePacks in packs.mjs). A version older than its
// pack's `latest` (an old run run again after a newer version shipped) is not published, since that would move
// `latest` back; the others go on, and the run fails naming it.
//
// In the release workflow npm authenticates through trusted publishing (`npm dist-tag` over it needs npm 11.21.0 or
// newer) and publishes with `--provenance` (failing when npm cannot attach an attestation). By hand (a pack's first
// publish), npm uses your login and publishes without provenance, which only CI can generate. Writes `released` (each
// `<name>@<version>:<dist-tag>` it released, space-separated, for dist-tags-check.mjs) to $GITHUB_OUTPUT when it is
// set.
//
//   [HELD="<id> ..."] node scripts/release/packs-publish.mjs <dir>
import { spawnSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { addDistTag, readLatestTag, readNextTag } from "./dist-tags.mjs";
import { heldPackNames, REGISTRY, releasePacks, releaseTag } from "./packs.mjs";

/**
 * Runs npm publish for one tarball; whether it succeeded, and its error output. stdin and stdout stay the terminal:
 * npm asks for a 2FA code or browser approval only when both are one. Its `npm error` lines, the publish conflict
 * included, go to stderr, which is captured and shown when npm exits.
 */
function publish(path, distTag) {
  const provenance = process.env.GITHUB_ACTIONS === "true" ? ["--provenance"] : [];
  const run = spawnSync("npm", ["publish", path, "--registry", REGISTRY, "--access", "public", "--tag", distTag, ...provenance], { stdio: ["inherit", "inherit", "pipe"], encoding: "utf8" });
  process.stderr.write(run.stderr ?? "");
  if (run.error !== undefined) throw run.error;
  return { ok: run.status === 0, output: run.stderr };
}

const [directory] = process.argv.slice(2);
if (directory === undefined) throw new Error("usage: packs-publish.mjs <dir written by packs-pack.ts>");
const out = resolve(directory);
const manifest = JSON.parse(readFileSync(join(out, "manifest.json"), "utf8"));

const { released, superseded } = await releasePacks(
  manifest,
  {
    publish: (file, distTag) => publish(join(out, file), distTag),
    readLatest: readLatestTag,
    readNext: readNextTag,
    distTag: addDistTag,
    log: (line) => console.log(line),
  },
  heldPackNames(process.env.HELD),
);
if (process.env.GITHUB_OUTPUT !== undefined) {
  appendFileSync(process.env.GITHUB_OUTPUT, `released=${released.map(({ name, version, distTag }) => `${name}@${version}:${distTag}`).join(" ")}\n`);
}
for (const { name, version, latest } of superseded) {
  console.log(`::error title=Superseded::${releaseTag(name, version)} is older than latest ${latest}, so it was not published; ${latest} supersedes it.`);
}
if (superseded.length > 0) process.exitCode = 1;
