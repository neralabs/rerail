#!/usr/bin/env node
// Points each named package's `next` dist-tag at the version named, one `latest` points at, unless `next` is already
// at a newer version (a preview past it), so the preview channel never trails the stable one (raiseNext in
// dist-tags.mjs). release.yml runs it for core after publishing under `latest` and after putting `latest` back;
// packs-publish.mjs and packs-promote.mjs do the same for each pack they publish or promote under `latest`. Running it
// again is safe.
//
// In the workflow npm moves the dist-tag through trusted publishing, which needs npm 11.21.0 or newer; by hand, npm
// uses your login.
//
//   node scripts/release/raise-next.mjs <name>@<version> ...
import { addDistTag, parseExpectation, raiseNext, readNextTag } from "./dist-tags.mjs";

const args = process.argv.slice(2);
if (args.length === 0) throw new Error("usage: raise-next.mjs <name>@<version> ...");
for (const { name, version, tag } of args.map(parseExpectation)) {
  if (tag !== "latest") throw new Error(`${name}@${version}:${tag}: only a version under latest raises next`);
  console.log(await raiseNext(name, version, { readNext: readNextTag, distTag: addDistTag }));
}
