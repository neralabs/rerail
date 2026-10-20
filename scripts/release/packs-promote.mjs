#!/usr/bin/env node
// Promotes pack versions from `next` to `latest`: points each pack's `latest` dist-tag at the version named. Only a
// version this repository released (its `<name>@<version>` tag exists) is promoted. One failure does not stop the
// others; the script exits non-zero naming each, and running it again is safe, since pointing `latest` where it
// already points changes nothing (promotePacks in packs.mjs). The release workflow passes what packs-promote-plan.mjs
// listed.
//
//   node scripts/release/packs-promote.mjs <id>@<version> ...
import { execFileSync } from "node:child_process";
import { listPacks, promotePacks, REGISTRY, releaseTags } from "./packs.mjs";

const packs = listPacks();
const specs = process.argv.slice(2).filter((argument) => argument !== "");
if (specs.length === 0) throw new Error("usage: packs-promote.mjs <id>@<version> ...");
const promotions = specs.map((spec) => {
  const at = spec.lastIndexOf("@");
  const pack = packs.find((candidate) => candidate.id === spec.slice(0, at));
  if (at <= 0 || pack === undefined) throw new Error(`${spec} is not <id>@<version> for a pack under packs/`);
  return { name: pack.name, version: spec.slice(at + 1) };
});

const npm = (args) => execFileSync("npm", [...args, "--registry", REGISTRY], { stdio: "inherit" });
const failures = promotePacks(promotions, releaseTags(), {
  promote: (name, version) => {
    npm(["dist-tag", "add", `${name}@${version}`, "latest"]);
    npm(["dist-tag", "ls", name]);
  },
  log: (line) => console.log(line),
});
for (const failure of failures) console.log(`::error title=Not promoted::${failure}`);
if (failures.length > 0) process.exit(1);
