#!/usr/bin/env node
// Which packs a release run tags and publishes: every published pack (not `"private": true`) whose package.json
// version has no release tag on GitHub yet, and every one tagged but not on npm (a run whose publish failed or was
// never approved), which any later run, a re-run of any job or the next push, publishes again (planPacks in
// packs.mjs). A pack npm cannot be read for fails the plan, so an unpublished version is never taken as released.
// A pack never published at all is reported as needing its first publish by hand (docs/publishing-a-new-pack.md) and left out.
// Every pack publishes under `next`, so a pack held at an older `latest` (put back on purpose) stays held.
// `--all` names every published pack without reading tags, for a pull request's dry run, which packs and checks but publishes
// nothing. Writes `publish` and `first` (space-separated pack ids) to $GITHUB_OUTPUT when it is set.
//
//   node scripts/release/packs-plan.mjs [--all]   (GH_REPO or the checkout's remote names the repository)
import { appendFileSync } from "node:fs";
import { readDistTags, readVersions } from "./dist-tags.mjs";
import { onRegistry, planPacks, publishedPacks, releaseTag, releaseTags } from "./packs.mjs";

const packs = publishedPacks();
const all = process.argv.includes("--all");
const packOf = (id) => packs.find((pack) => pack.id === id);

let plan = { publish: packs.map((pack) => pack.id), first: [], unpublished: [], unreadable: [] };
if (!all) {
  const tags = releaseTags();
  // Only a version already tagged needs npm read: whether it is there decides if it is released.
  const tagged = packs.filter((pack) => tags.includes(releaseTag(pack.name, pack.version))).map((pack) => pack.name);
  const [distTags, versions] = await Promise.all([readDistTags(tagged, { fresh: true }), readVersions(tagged)]);
  plan = planPacks(packs, tags, (name, version) => onRegistry(version, distTags.get(name) ?? null, versions.get(name) ?? null));
  if (plan.unreadable.length > 0) {
    throw new Error(`could not read ${plan.unreadable.join(", ")} from npm, so whether each tagged version is published is unknown`);
  }
}
const { publish, first, unpublished } = plan;

for (const id of publish) {
  console.log(`publish ${id} ${packOf(id).version}${unpublished.includes(id) ? " (tagged but not on npm: published again)" : ""}`);
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
  appendFileSync(process.env.GITHUB_OUTPUT, `publish=${publish.join(" ")}\nfirst=${first.join(" ")}\n`);
}
