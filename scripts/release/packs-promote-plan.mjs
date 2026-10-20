#!/usr/bin/env node
// Which pack versions a promotion moves from `next` to `latest`: every pack whose package.json version was released
// (its `<name>@<version>` tag exists) and is not `latest` on npm yet (planPromotions in packs.mjs). With a pack id,
// only that pack, at the version given or else its package.json one. Writes `promote` (space-separated
// `<id>@<version>`) to $GITHUB_OUTPUT and a table to $GITHUB_STEP_SUMMARY when they are set, so the run shows what
// the promote job will move before anyone approves it.
//
// The packs are restricted, so reading their `latest` needs an npm login, or NODE_AUTH_TOKEN with a read token.
//
//   node scripts/release/packs-promote-plan.mjs [<id> [<version>]]   (GH_REPO or the checkout's remote names the repository)
import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { listPacks, planPromotions, REGISTRY, releaseTags } from "./packs.mjs";

/** The version `name`'s `latest` dist-tag names, or null when it names none. */
function latestOf(name) {
  const latest = execFileSync("npm", ["view", name, "dist-tags.latest", "--registry", REGISTRY], { encoding: "utf8" }).trim();
  return latest === "" ? null : latest;
}

const [id, version] = process.argv.slice(2).filter((argument) => argument !== "");
const packs = listPacks();
let candidates = packs;
if (id !== undefined) {
  const pack = packs.find((candidate) => candidate.id === id);
  if (pack === undefined) throw new Error(`${id} is not a pack under packs/`);
  candidates = [{ ...pack, version: version ?? pack.version }];
}
const { promote, unreleased } = planPromotions(candidates, releaseTags(), latestOf);

for (const pack of promote) console.log(`promote ${pack.id} ${pack.latest ?? "(none)"} -> ${pack.version}`);
for (const pack of unreleased) console.log(`::warning title=Not released::${pack.id} ${pack.version} has no release tag, so it is not promoted.`);
if (promote.length === 0) console.log("Every released pack version is latest already.");

if (process.env.GITHUB_STEP_SUMMARY !== undefined) {
  const lines = promote.length === 0
    ? ["Every released pack version is `latest` already; nothing to promote."]
    : ["The promote job points `latest` at these versions:", "", "| Pack | `latest` now | Promoted to |", "|---|---|---|",
      ...promote.map((pack) => `| ${pack.name} | ${pack.latest ?? "none"} | ${pack.version} |`)];
  if (unreleased.length > 0) lines.push("", `Not released, so not promoted: ${unreleased.map((pack) => `${pack.id} ${pack.version}`).join(", ")}.`);
  appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${lines.join("\n")}\n`);
}
if (process.env.GITHUB_OUTPUT !== undefined) appendFileSync(process.env.GITHUB_OUTPUT, `promote=${promote.map((pack) => `${pack.id}@${pack.version}`).join(" ")}\n`);
