#!/usr/bin/env node
// Which pack versions a promotion moves from `next` to `latest` (promotionRequest and planPromotions in packs.mjs).
// With no pack id, every pack whose package.json version is released (its `<name>@<version>` tag exists) and is not
// `latest` yet, less any pack held at its `latest` (put back on purpose). With an id, that pack alone, at the version
// given or else its package.json one; naming an older version holds the pack there. Writes `promote` (a JSON list
// for packs-promote.mjs) to $GITHUB_OUTPUT and a table to $GITHUB_STEP_SUMMARY when they are set, so the run shows
// what the promote job will move before anyone approves it.
//
// Exits non-zero, after writing both, when a named version was never released or a pack's dist-tags could not be
// read: the others are still listed and promoted.
//
// The packs are restricted, so reading their dist-tags needs an npm login, or NODE_AUTH_TOKEN with a read token.
//
//   node scripts/release/packs-promote-plan.mjs [<id> [<version>]]   (an empty argument is no value; GH_REPO or the
//   checkout's remote names the repository)
import { execFile } from "node:child_process";
import { appendFileSync } from "node:fs";
import { promisify } from "node:util";
import { HOLD_TAG, listPacks, planPromotions, promotionRequest, REGISTRY, releaseTag, releaseTags } from "./packs.mjs";

/** Each name's dist-tags, read in parallel; null for one npm could not give, with why on stderr. */
async function readDistTags(names) {
  const read = async (name) => {
    try {
      const { stdout } = await promisify(execFile)("npm", ["view", name, "dist-tags", "--json", "--registry", REGISTRY]);
      return [name, JSON.parse(stdout)];
    } catch (error) {
      console.error(`${name}: ${error instanceof Error ? error.message : String(error)}`);
      return [name, null];
    }
  };
  return new Map(await Promise.all(names.map(read)));
}

// The workflow passes both inputs, empty when not given: read them by position.
const [id, version] = process.argv.slice(2, 4).map((argument) => (argument === "" ? undefined : argument));
const request = promotionRequest(listPacks(), id, version);
const tags = releaseTags();
const released = request.packs.filter((pack) => tags.includes(releaseTag(pack.name, pack.version)));
const distTags = await readDistTags(released.map((pack) => pack.name));
const plan = planPromotions(request.packs, tags, (name) => distTags.get(name) ?? null, { named: request.named });

const note = (pack) => (pack.hold ? `put back; held here (\`${HOLD_TAG}\`)` : pack.release ? `releases its hold at ${distTags.get(pack.name)?.[HOLD_TAG]}` : "");
for (const pack of plan.promote) console.log(`promote ${pack.id} ${pack.latest ?? "(none)"} -> ${pack.version} ${note(pack)}`.trimEnd());
for (const pack of plan.held) console.log(`held ${pack.id} at ${pack.latest}: not promoted`);
for (const pack of plan.unreleased) {
  // A named version that was never released is a mistake to report; the sweep just has nothing to do for it.
  console.log(`::${request.named ? "error" : "warning"} title=Not released::${pack.id} ${pack.version} has no release tag, so it is not promoted.`);
}
for (const pack of plan.unreadable) console.log(`::error title=Not read::${pack.name}'s dist-tags could not be read from npm, so it is not promoted.`);
if (plan.promote.length === 0) console.log("Nothing to promote.");

if (process.env.GITHUB_STEP_SUMMARY !== undefined) {
  const lines = plan.promote.length === 0
    ? ["Nothing to promote."]
    : ["The promote job points `latest` at these versions:", "", "| Pack | `latest` now | Promoted to | |", "|---|---|---|---|",
      ...plan.promote.map((pack) => `| ${pack.name} | ${pack.latest ?? "none"} | ${pack.version} | ${note(pack)} |`)];
  if (plan.held.length > 0) {
    lines.push("", `Held where someone put them back, so not promoted (run with that pack's id to promote it): ${plan.held.map((pack) => `${pack.id} at ${pack.latest}`).join(", ")}.`);
  }
  if (plan.unreleased.length > 0) lines.push("", `Not released, so not promoted: ${plan.unreleased.map((pack) => `${pack.id} ${pack.version}`).join(", ")}.`);
  if (plan.unreadable.length > 0) lines.push("", `Could not be read from npm, so not promoted: ${plan.unreadable.map((pack) => pack.id).join(", ")}.`);
  appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${lines.join("\n")}\n`);
}
const promotions = plan.promote.map(({ id: packId, version: packVersion, hold, release }) => ({ id: packId, version: packVersion, hold, release }));
if (process.env.GITHUB_OUTPUT !== undefined) appendFileSync(process.env.GITHUB_OUTPUT, `promote=${promotions.length === 0 ? "" : JSON.stringify(promotions)}\n`);
if (plan.unreadable.length > 0 || (request.named && plan.unreleased.length > 0)) process.exitCode = 1;
