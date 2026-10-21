#!/usr/bin/env node
// Which packs a release run publishes: every pack whose package.json version has no release tag on GitHub yet.
// A pack never published at all is reported as needing its first publish by hand (docs/publishing-a-new-pack.md) and left out.
// Of those published, a pack held at an older `latest` (heldPacks in packs.mjs) is published under `next`, so the hold
// stays; reading that from npm needs NODE_AUTH_TOKEN with a read token, and a pack whose dist-tags cannot be read
// fails the plan, so nothing publishes over a hold unseen.
// `--all` names every pack without reading tags, for a pull request's dry run, which packs and checks but publishes
// nothing. Writes `publish`, `first` and `held` (space-separated pack ids) to $GITHUB_OUTPUT when it is set.
//
//   node scripts/release/packs-plan.mjs [--all]   (GH_REPO or the checkout's remote names the repository)
import { appendFileSync } from "node:fs";
import { readDistTags } from "./dist-tags.mjs";
import { heldPacks, listPacks, planPacks, releaseTags } from "./packs.mjs";

const packs = listPacks();
const all = process.argv.includes("--all");
const { publish, first } = all ? { publish: packs.map((pack) => pack.id), first: [] } : planPacks(packs, releaseTags());
const packOf = (id) => packs.find((pack) => pack.id === id);

let held = [];
if (!all && publish.length > 0) {
  const distTags = await readDistTags(publish.map((id) => packOf(id).name));
  const read = heldPacks(publish.map((id) => packOf(id).name), (name) => distTags.get(name) ?? null);
  if (read.unreadable.length > 0) {
    throw new Error(`could not read the dist-tags of ${read.unreadable.join(", ")} from npm, so whether each is held is unknown`);
  }
  held = publish.filter((id) => read.held.includes(packOf(id).name));
}

for (const id of publish) console.log(`publish ${id} ${packOf(id).version}${held.includes(id) ? " under next: held at an older latest" : ""}`);
for (const id of first) {
  console.log(
    `::error title=First publish needed::${id} ${packOf(id).version} has never been published. Publish it by hand and configure ` +
      "its trusted publisher (docs/publishing-a-new-pack.md); the next release run publishes its later versions.",
  );
}
if (publish.length === 0 && first.length === 0) console.log("Every pack's version is published.");
if (process.env.GITHUB_OUTPUT !== undefined) {
  appendFileSync(process.env.GITHUB_OUTPUT, `publish=${publish.join(" ")}\nfirst=${first.join(" ")}\nheld=${held.join(" ")}\n`);
}
