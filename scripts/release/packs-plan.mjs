#!/usr/bin/env node
// Which packs a release run tags and publishes: every published pack (not `"private": true`) whose package.json
// version has no release tag on GitHub yet, and every one tagged but not on npm (a run whose publish failed or was
// never approved), which any later run, a re-run of any job or the next push, publishes again (planPacks in
// packs.mjs). A pack npm cannot be read for fails the plan, so an unpublished version is never taken as released.
// A pack never published at all is reported as needing its first publish by hand (docs/publishing-a-new-pack.md) and left out.
// Of those published, a pack held at an older `latest` (heldPacks in packs.mjs) is published under `next`, so the hold
// stays; reading that from npm needs NODE_AUTH_TOKEN with a read token, and a pack whose dist-tags cannot be read
// fails the plan, so nothing publishes over a hold unseen.
// `--all` names every published pack without reading tags, for a pull request's dry run, which packs and checks but publishes
// nothing. Writes `publish`, `first` and `held` (space-separated pack ids) to $GITHUB_OUTPUT when it is set.
//
//   node scripts/release/packs-plan.mjs [--all]   (GH_REPO or the checkout's remote names the repository)
import { appendFileSync } from "node:fs";
import { readDistTags, readVersions } from "./dist-tags.mjs";
import { heldPacks, onRegistry, planPacks, publishedPacks, releaseTag, releaseTags } from "./packs.mjs";

const packs = publishedPacks();
const all = process.argv.includes("--all");
const packOf = (id) => packs.find((pack) => pack.id === id);

let plan = { publish: packs.map((pack) => pack.id), first: [], unpublished: [], unreadable: [] };
let distTags = new Map();
if (!all) {
  const tags = releaseTags();
  // Only a version already tagged needs npm read: whether it is there decides if it is released.
  const tagged = packs.filter((pack) => tags.includes(releaseTag(pack.name, pack.version))).map((pack) => pack.name);
  const [readTags, versions] = await Promise.all([readDistTags(tagged, { fresh: true }), readVersions(tagged)]);
  distTags = readTags;
  plan = planPacks(packs, tags, (name, version) => onRegistry(version, distTags.get(name) ?? null, versions.get(name) ?? null));
  if (plan.unreadable.length > 0) {
    throw new Error(`could not read ${plan.unreadable.join(", ")} from npm, so whether each tagged version is published is unknown`);
  }
}
const { publish, first, unpublished } = plan;

let held = [];
if (!all && publish.length > 0) {
  const unread = publish.map((id) => packOf(id).name).filter((name) => !distTags.has(name));
  for (const [name, read] of await readDistTags(unread, { fresh: true })) distTags.set(name, read);
  const read = heldPacks(publish.map((id) => packOf(id).name), (name) => distTags.get(name) ?? null);
  if (read.unreadable.length > 0) {
    throw new Error(`could not read the dist-tags of ${read.unreadable.join(", ")} from npm, so whether each is held is unknown`);
  }
  held = publish.filter((id) => read.held.includes(packOf(id).name));
}

for (const id of publish) {
  const why = [unpublished.includes(id) ? "tagged but not on npm: published again" : "", held.includes(id) ? "under next: held at an older latest" : ""].filter(Boolean);
  console.log(`publish ${id} ${packOf(id).version}${why.length > 0 ? ` (${why.join("; ")})` : ""}`);
  if (unpublished.includes(id)) console.log(`::warning title=Tagged but not on npm::${releaseTag(packOf(id).name, packOf(id).version)} is tagged but npm does not have it; this run publishes it.`);
}
for (const id of first) {
  console.log(
    `::error title=First publish needed::${id} ${packOf(id).version} has never been published. Publish it by hand and configure ` +
      "its trusted publisher (docs/publishing-a-new-pack.md); the next release run publishes its later versions.",
  );
}
if (publish.length === 0 && first.length === 0) console.log("Every pack's version is tagged and on npm.");
if (process.env.GITHUB_OUTPUT !== undefined) {
  appendFileSync(process.env.GITHUB_OUTPUT, `publish=${publish.join(" ")}\nfirst=${first.join(" ")}\nheld=${held.join(" ")}\n`);
}
