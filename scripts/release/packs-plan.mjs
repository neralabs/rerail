#!/usr/bin/env node
// Which packs a release run publishes: every pack whose package.json version has no release tag on GitHub yet.
// A pack never published at all is reported as needing its first publish by hand (docs/publishing-a-new-pack.md) and left out.
// `--all` names every pack without reading tags, for a pull request's dry run, which packs and checks but publishes
// nothing. Writes `publish` and `first` (space-separated pack ids) to $GITHUB_OUTPUT when it is set.
//
//   node scripts/release/packs-plan.mjs [--all]   (GH_REPO or the checkout's remote names the repository)
import { appendFileSync } from "node:fs";
import { listPacks, planPacks, releaseTags } from "./packs.mjs";

const packs = listPacks();
const { publish, first } = process.argv.includes("--all") ? { publish: packs.map((pack) => pack.id), first: [] } : planPacks(packs, releaseTags());
const versionOf = (id) => packs.find((pack) => pack.id === id).version;

for (const id of publish) console.log(`publish ${id} ${versionOf(id)}`);
for (const id of first) {
  console.log(
    `::error title=First publish needed::${id} ${versionOf(id)} has never been published. Publish it by hand and configure ` +
      "its trusted publisher (docs/publishing-a-new-pack.md); the next release run publishes its later versions.",
  );
}
if (publish.length === 0 && first.length === 0) console.log("Every pack's version is published.");
if (process.env.GITHUB_OUTPUT !== undefined) appendFileSync(process.env.GITHUB_OUTPUT, `publish=${publish.join(" ")}\nfirst=${first.join(" ")}\n`);
