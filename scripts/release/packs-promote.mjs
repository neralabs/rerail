#!/usr/bin/env node
// Promotes pack versions from `next` to `latest`, as packs-promote-plan.mjs listed them in $PROMOTE (a JSON list of
// `{ id, version, hold, sweep }`): points each pack's `latest` dist-tag at its version, and for a pack put back on
// purpose also sets `held` there, or for one promoted past its hold removes `held`; then points `next` at the version
// too unless `next` is already at a newer one, so `next` never trails `latest`. Only a version this repository
// released (its `<name>@<version>` tag exists) is promoted. Unless it is a hold, it is checked as this runs, after
// the approval: a sweep's pack held meanwhile is left alone, and one `latest` has passed, or one the core on `latest`
// can't load (promote core first), is refused. One
// failure does not stop the others; the script exits non-zero naming each, and running it again is safe (promotePacks
// in packs.mjs). It runs with no node_modules, in a job that can mint an npm publish credential. Writes `promoted` (each
// `<name>@<version>` it promoted, space-separated, for dist-tags-check.mjs) to $GITHUB_OUTPUT when it is set.
//
//   PROMOTE='[{"id":"python","version":"0.1.3","hold":true,"sweep":false}]' node scripts/release/packs-promote.mjs
import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { addDistTag, readDistTagsOrNone, readNextTag, readPublishedManifest } from "./dist-tags.mjs";
import { HOLD_TAG, promotePacks, publishedPacks, REGISTRY, releaseTags } from "./packs.mjs";

/** Core's npm name, as core.mjs has it; this job installs no node_modules, which core.mjs needs. */
const CORE_NAME = "@neralabs/rerail";
const packs = publishedPacks();
const listed = JSON.parse(process.env.PROMOTE ?? "[]");
if (!Array.isArray(listed) || listed.length === 0) throw new Error("PROMOTE names no pack to promote");
const promotions = listed.map(({ id, version, hold, sweep }) => {
  const pack = packs.find((candidate) => candidate.id === id);
  if (pack === undefined || typeof version !== "string") throw new Error(`${JSON.stringify({ id, version })} is not a published pack under packs/ with a version`);
  return { name: pack.name, version, hold: hold === true, sweep: sweep === true };
});

const { promoted, held, failures } = await promotePacks(promotions, releaseTags(), {
  promote: (name, version) => addDistTag(name, version, "latest"),
  hold: (name, version) => addDistTag(name, version, HOLD_TAG),
  release: (name) => execFileSync("npm", ["dist-tag", "rm", name, HOLD_TAG, "--registry", REGISTRY], { stdio: "inherit" }),
  readNext: readNextTag,
  readDistTags: readDistTagsOrNone,
  readCoreRange: async (name, version) => (await readPublishedManifest(name, version)).peerDependencies?.[CORE_NAME],
  readCoreLatest: async () => {
    const { latest } = await readDistTagsOrNone(CORE_NAME);
    if (latest === undefined) throw new Error(`${CORE_NAME} has no latest, so whether it loads the pack is unknown`);
    return latest;
  },
  distTag: addDistTag,
  log: (line) => console.log(line),
});
if (process.env.GITHUB_OUTPUT !== undefined) {
  appendFileSync(process.env.GITHUB_OUTPUT, `promoted=${promoted.map(({ name, version }) => `${name}@${version}`).join(" ")}\n`);
}
for (const { name, version, at } of held) {
  console.log(`::notice title=Held::${name}@${version} was not promoted: someone put ${name} back to ${at} and held it there while this waited. Promote it by name to release the hold.`);
}
for (const failure of failures) console.log(`::error title=Not promoted::${failure}`);
if (failures.length > 0) process.exit(1);
