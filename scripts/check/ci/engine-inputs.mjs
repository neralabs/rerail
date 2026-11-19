#!/usr/bin/env node
// A hash of everything the release engine is built from, so a CI build job whose inputs match an engine an earlier
// run built downloads that engine instead of compiling it (.github/actions/engine-artifact). The inputs, by their
// committed blob ids (`git ls-files --stage`: the same on every OS, whatever a checkout does to line ends or times):
//   - the crates the engine builds and their build scripts: native/ (the vendored dependencies and grammars with it),
//     accel/, tools/bitlex/;
//   - the language registry the build script generates tables from (languages/);
//   - the lockfile, the workspace manifest and its profiles, cargo's configuration, the toolchain, the checkout's line
//     ends (.gitattributes);
//   - the release build's script and this one, and the napi CLI and cross toolchain versions package-lock.json pins;
//   - every file outside those that an engine source embeds (`include_str!`/`include_bytes!` with a literal path:
//     packs' rules, docs' `.rerail` libraries, capabilities.json, test fixtures), found by reading the sources, so an
//     embed added later counts without a change here.
// A path that changes none of these (TypeScript, docs, the rest of packs/) leaves the hash, and the engine, as they were.
//
//   node scripts/check/ci/engine-inputs.mjs           the hash (hex) on stdout
//   node scripts/check/ci/engine-inputs.mjs --list    the entries it hashes, one per line
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ENGINE_DIRS = ["native/", "accel/", "tools/bitlex/", "languages/"];
export const ENGINE_FILES = [
  "Cargo.lock",
  "Cargo.toml",
  ".cargo/config.toml",
  "rust-toolchain.toml",
  ".gitattributes",
  "scripts/build/release-engine.mjs",
  "scripts/check/ci/engine-inputs.mjs",
];
const LOCKED_PACKAGES = ["node_modules/@napi-rs/cli", "node_modules/@napi-rs/cross-toolchain"];
const EMBED = /include_(?:str|bytes)!\s*\(\s*"([^"]+)"\s*\)/gu;

/** Every tracked file's blob id, by repository-relative path. */
function trackedBlobs(root) {
  const git = spawnSync("git", ["ls-files", "--stage", "-z"], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  if (git.status !== 0) throw new Error(`git ls-files failed: ${git.stderr}`);
  const blobs = new Map();
  for (const entry of git.stdout.split("\0")) {
    if (entry === "") continue;
    const tab = entry.indexOf("\t");
    blobs.set(entry.slice(tab + 1), entry.slice(0, tab).split(" ")[1]);
  }
  return blobs;
}

/** The entries the hash covers, sorted: `<blob> <path>` per file, then the locked tool versions. */
export function engineInputs(root) {
  const blobs = trackedBlobs(root);
  const inEngine = (path) => ENGINE_DIRS.some((dir) => path.startsWith(dir));
  const paths = new Set(ENGINE_FILES.filter((path) => blobs.has(path)));
  for (const path of blobs.keys()) {
    if (!inEngine(path)) continue;
    paths.add(path);
    if (!path.endsWith(".rs") || path.startsWith("native/vendor/")) continue;
    for (const [, target] of readFileSync(`${root}/${path}`, "utf8").matchAll(EMBED)) {
      const embedded = posix.normalize(posix.join(posix.dirname(path), target));
      if (blobs.has(embedded)) paths.add(embedded);
    }
  }
  const lock = JSON.parse(readFileSync(`${root}/package-lock.json`, "utf8"));
  return [
    ...[...paths].sort().map((path) => `${blobs.get(path)} ${path}`),
    ...LOCKED_PACKAGES.map((name) => `${lock.packages?.[name]?.version ?? "absent"} ${name}`),
  ];
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = dirname(dirname(dirname(dirname(fileURLToPath(import.meta.url)))));
  const entries = engineInputs(root);
  if (process.argv.includes("--list")) console.log(entries.join("\n"));
  else console.log(createHash("sha256").update(entries.join("\n")).digest("hex"));
}
