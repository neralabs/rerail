#!/usr/bin/env node
// Promotes pack versions from `next` to `latest`, as packs-promote-plan.mjs listed them in $PROMOTE (a JSON list of
// `{ id, version, hold, release }`): points each pack's `latest` dist-tag at its version, and for a pack put back on
// purpose also sets `held` there, or for one promoted past its hold removes `held`. Only a version this repository
// released (its `<name>@<version>` tag exists) is promoted. One failure does not stop the others; the script exits
// non-zero naming each, and running it again is safe (promotePacks in packs.mjs).
//
//   PROMOTE='[{"id":"python","version":"0.1.3","hold":true,"release":false}]' node scripts/release/packs-promote.mjs
import { execFileSync } from "node:child_process";
import { HOLD_TAG, listPacks, promotePacks, REGISTRY, releaseTags } from "./packs.mjs";

const packs = listPacks();
const listed = JSON.parse(process.env.PROMOTE ?? "[]");
if (!Array.isArray(listed) || listed.length === 0) throw new Error("PROMOTE names no pack to promote");
const promotions = listed.map(({ id, version, hold, release }) => {
  const pack = packs.find((candidate) => candidate.id === id);
  if (pack === undefined || typeof version !== "string") throw new Error(`${JSON.stringify({ id, version })} is not a pack under packs/ with a version`);
  return { name: pack.name, version, hold: hold === true, release: release === true };
});

const npm = (args) => execFileSync("npm", [...args, "--registry", REGISTRY], { stdio: "inherit" });
const failures = promotePacks(promotions, releaseTags(), {
  promote: (name, version) => npm(["dist-tag", "add", `${name}@${version}`, "latest"]),
  hold: (name, version) => npm(["dist-tag", "add", `${name}@${version}`, HOLD_TAG]),
  release: (name) => npm(["dist-tag", "rm", name, HOLD_TAG]),
  log: (line) => console.log(line),
});
// What each pack's dist-tags are now, for the log; a listing that fails says so and the run goes on to its report.
for (const { name } of promotions) {
  try {
    npm(["dist-tag", "ls", name]);
  } catch {
    console.log(`::warning title=Not listed::could not list ${name}'s dist-tags`);
  }
}
if (failures.length > 0) process.exit(1);
