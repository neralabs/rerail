#!/usr/bin/env node
// A hash of everything the release engine is built from, so a CI build job whose inputs match an engine an earlier
// run built downloads that engine instead of compiling it (.github/actions/engine-artifact). The inputs, by their
// committed blob ids (`git ls-files --stage`: the same on every OS, whatever a checkout does to line ends or times):
//   - the crates the engine builds and their build scripts: native/ (the vendored dependencies and grammars with it)
//     and every crate a manifest reaches from it by a `path` (accel/, forest/, tools/bitlex/ today), read from the
//     manifests, so a crate added later counts without a change here, but for the Markdown the build never reads
//     (`unreadDoc`);
//   - the language registry the build script generates tables from (languages/);
//   - the lockfile, the workspace manifest and its profiles, cargo's configuration, the toolchain, the checkout's line
//     ends (.gitattributes);
//   - the release build's script and this one, package.json's `napi` block (the engine file's name and the loader
//     napi writes), and the napi CLI and cross toolchain versions package-lock.json pins;
//   - every file that an engine source embeds (`include_str!`/`include_bytes!` with a literal path: packs' rules, docs'
//     `.rerail` libraries, capabilities.json, test fixtures, a vendored crate's README as its docs), found by reading
//     the sources, so an embed added later counts without a change here.
// A path that changes none of these (TypeScript, docs, the rest of packs/, a README beside the crates) leaves the hash,
// and the engine, as they were. test-selection.mjs reads the same inputs (`engineSources`, `isEngineInput`), every file
// of the engine's directories included: a change to one is a change to the engine the tests load.
//
//   node scripts/check/ci/engine-inputs.mjs           the hash (hex) on stdout
//   node scripts/check/ci/engine-inputs.mjs --list    the entries it hashes, one per line
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The engine's crate, whose manifest the other crates are reached from, and the data its build script reads. */
export const ENGINE_ROOTS = ["native/", "languages/"];
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
const PATH_KEY = /\bpath\s*=\s*"([^"]+)"/gu;

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

/** The crate directories (`dir/`) a manifest names by `path`: dependencies of every kind and patches. */
function pathCrates(root, manifest, blobs) {
  const dir = posix.dirname(manifest);
  const crates = [];
  for (const [, target] of readFileSync(`${root}/${manifest}`, "utf8").matchAll(PATH_KEY)) {
    const crate = posix.normalize(posix.join(dir, target));
    if (blobs.has(`${crate}/Cargo.toml`)) crates.push(`${crate}/`);
  }
  return crates;
}

/**
 * The directories the engine is built from: native/, languages/ and every crate directory the workspace manifest or
 * a reached crate's manifest names by `path`, transitively.
 */
function engineDirs(root, blobs) {
  const dirs = new Set(ENGINE_ROOTS);
  const manifests = ["Cargo.toml", "native/Cargo.toml"].filter((path) => blobs.has(path));
  for (let next = manifests.pop(); next !== undefined; next = manifests.pop()) {
    for (const crate of pathCrates(root, next, blobs)) {
      if (dirs.has(crate)) continue;
      dirs.add(crate);
      manifests.push(`${crate}Cargo.toml`);
    }
  }
  return [...dirs].sort();
}

/**
 * Whether `path`, in the engine's directories, is Markdown the build never reads. The build scripts read named files,
 * `*.json` from languages/ (native/src/build_languages.rs), and every file under native/src/, which native/build.rs
 * hashes into the engine's build id (`collect_sources`, RERAIL_ENGINE_BUILD_ID), notes included; tree-sitter's build
 * script lists its src/ only to watch it, and compiles lib.c. A Markdown file a source embeds stays an input whatever
 * this says: the embed scan adds it.
 */
export function unreadDoc(path) {
  return path.endsWith(".md") && !path.startsWith("native/src/");
}

/** The tracked files `path`, an engine source (a vendored one included), embeds by a literal path. */
function embeds(root, path, blobs) {
  if (!path.endsWith(".rs")) return [];
  const found = [];
  for (const [, target] of readFileSync(`${root}/${path}`, "utf8").matchAll(EMBED)) {
    const embedded = posix.normalize(posix.join(posix.dirname(path), target));
    if (blobs.has(embedded)) found.push(embedded);
  }
  return found;
}

/**
 * What the engine is built from: `dirs`, the directories (`dir/`) whose files are inputs, but for the Markdown the
 * build never reads (`unreadDoc`), and `paths`, every tracked input file (those directories' files, the single files,
 * the files their sources embed).
 */
export function engineSources(root, blobs = trackedBlobs(root)) {
  const dirs = engineDirs(root, blobs);
  const paths = new Set(ENGINE_FILES.filter((path) => blobs.has(path)));
  for (const path of blobs.keys()) {
    if (!dirs.some((dir) => path.startsWith(dir))) continue;
    if (!unreadDoc(path)) paths.add(path);
    for (const embedded of embeds(root, path, blobs)) paths.add(embedded);
  }
  return { dirs, paths };
}

/** Whether a change to `path` (repository-relative, tracked or deleted) changes the engine `sources` describes. */
export function isEngineInput(path, sources) {
  return sources.paths.has(path) || ENGINE_FILES.includes(path) || sources.dirs.some((dir) => path.startsWith(dir));
}

/** package.json's `napi` block, which names the engine file and the loader napi writes beside it. */
function napiConfig(root) {
  return JSON.stringify(JSON.parse(readFileSync(`${root}/package.json`, "utf8")).napi ?? null);
}

/**
 * The entries the hash covers: `<blob> <path>` per file, sorted, then package.json's `napi` block and the locked
 * tool versions.
 */
export function engineInputs(root) {
  const blobs = trackedBlobs(root);
  const { paths } = engineSources(root, blobs);
  const lock = JSON.parse(readFileSync(`${root}/package-lock.json`, "utf8"));
  return [
    ...[...paths].sort().map((path) => `${blobs.get(path)} ${path}`),
    `${napiConfig(root)} package.json#napi`,
    ...LOCKED_PACKAGES.map((name) => `${lock.packages?.[name]?.version ?? "absent"} ${name}`),
  ];
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = dirname(dirname(dirname(dirname(fileURLToPath(import.meta.url)))));
  const entries = engineInputs(root);
  if (process.argv.includes("--list")) console.log(entries.join("\n"));
  else console.log(createHash("sha256").update(entries.join("\n")).digest("hex"));
}
