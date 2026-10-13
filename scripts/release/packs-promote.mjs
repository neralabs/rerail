#!/usr/bin/env node
// Promotes one pack version from `next` to `latest`: points the pack's `latest` dist-tag at it. Only a version this
// repository released (its `<name>@<version>` tag exists) can be promoted. The version defaults to the one in
// packs/<id>/package.json.
//
//   node scripts/release/packs-promote.mjs <id> [version]
import { execFileSync } from "node:child_process";
import { listPacks, REGISTRY, releaseTag, releaseTags } from "./packs.mjs";

const [id, requested] = process.argv.slice(2);
const pack = listPacks().find((candidate) => candidate.id === id);
if (pack === undefined) throw new Error(`usage: packs-promote.mjs <id> [version]; ${id ?? "no id"} is not a pack under packs/`);
const version = requested === undefined || requested === "" ? pack.version : requested;
const tag = releaseTag(pack.name, version);
if (!releaseTags().includes(tag)) throw new Error(`${tag} was never released from this repository (no such git tag)`);

const npm = (args) => execFileSync("npm", [...args, "--registry", REGISTRY], { stdio: "inherit" });
npm(["dist-tag", "add", `${pack.name}@${version}`, "latest"]);
npm(["dist-tag", "ls", pack.name]);
