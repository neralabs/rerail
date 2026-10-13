#!/usr/bin/env node
// Sets each pack workspace's version in package-lock.json to its package.json's, after `changeset version` bumps
// the packs and leaves the lockfile behind. Touches nothing else, so it needs no registry and moves no dependency.
//
//   node scripts/release/sync-pack-lockfile.mjs   (the second half of npm run version:packs)
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { listPacks, ROOT, syncPackLockfile } from "./packs.mjs";

const path = join(ROOT, "package-lock.json");
const before = readFileSync(path, "utf8");
const after = `${JSON.stringify(syncPackLockfile(JSON.parse(before), listPacks()), null, 2)}\n`;
if (after !== before) writeFileSync(path, after);
console.log(after === before ? "sync-pack-lockfile: package-lock.json already records every pack's version" : "sync-pack-lockfile: package-lock.json updated");
