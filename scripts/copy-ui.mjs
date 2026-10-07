#!/usr/bin/env node
//
// The page is plain HTML, CSS, and ES modules; "building" it is a copy into the directory
// the daemon serves. Removing the destination first makes deletions in `ui/` propagate, so
// a stale asset can never outlive the source it came from. Only page assets ship: dotfiles
// (`.DS_Store`, editor state) and any stray `node_modules` are filtered out of the copy.

import { cp, mkdir, rm } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const source = join(packageRoot, "ui");
const destination = join(packageRoot, "dist", "ui");

await rm(destination, { recursive: true, force: true });
await mkdir(dirname(destination), { recursive: true });
const shipsAsset = (path) => {
  const name = basename(path);
  return name !== "node_modules" && !name.startsWith(".");
};

await cp(source, destination, { recursive: true, filter: shipsAsset });
process.stdout.write(`copied ${source} to ${destination}\n`);
