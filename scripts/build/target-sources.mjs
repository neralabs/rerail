// Lets cargo reuse a target directory CI restores from main's artifact (.github/actions/cargo-target). Cargo decides
// a workspace crate (and a build script's `rerun-if-changed` paths) is stale when a source file is newer than the
// crate's last build, by modification time; a fresh checkout gives every file the checkout's time, so a restored
// target rebuilt every workspace crate. Only dependencies from crates.io, fingerprinted by version, were reused.
//
//   node scripts/build/target-sources.mjs record <target-dir>   after a build, before the target is saved
//   node scripts/build/target-sources.mjs apply <target-dir>    after the checkout and the target's restore
//
// `record` writes the checkout's tracked files with their blob ids (`git ls-files -s`) into the target directory, so
// the list is saved and restored with it. `apply` compares the checkout with that list and sets every file whose blob
// is unchanged, and every directory whose tracked entries are the same, to a time older than any build. Changed and
// added files keep the checkout's time, newer than the restored outputs, so cargo rebuilds what they reach and
// nothing else. A directory that gained or lost a tracked file keeps its time too: `rerun-if-changed` on a directory
// reads the newest time of anything under it, a removal included. Without a list (a cold target), `apply` does
// nothing.
import { spawnSync } from "node:child_process";
import { existsSync, lutimesSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const LIST = ".rerail-sources";
/** 2001-09-09: older than any build output a restored target holds. */
const UNCHANGED_TIME = 1_000_000_000;

/** The checkout's tracked files as `git ls-files --stage -z` lists them. */
function stagedList() {
  const git = spawnSync("git", ["ls-files", "--stage", "-z"], { encoding: "utf8", maxBuffer: 1 << 28 });
  if (git.status !== 0) throw new Error(`git ls-files failed: ${git.stderr}`);
  return git.stdout;
}

/** Path to blob id from `<mode> <blob> <stage>\t<path>` entries, NUL-separated; submodules left out. */
function parseList(text) {
  const blobs = new Map();
  for (const entry of text.split("\0")) {
    if (entry === "") continue;
    const tab = entry.indexOf("\t");
    const [mode, blob] = entry.slice(0, tab).split(" ");
    if (mode === "160000") continue;
    blobs.set(entry.slice(tab + 1), blob);
  }
  return blobs;
}

function ancestors(path) {
  const directories = [];
  for (let directory = dirname(path); directory !== "."; directory = dirname(directory)) directories.push(directory);
  return directories;
}

function record(target) {
  const list = stagedList();
  writeFileSync(join(target, LIST), list);
  console.log(`Recorded ${parseList(list).size} tracked files in ${join(target, LIST)}`);
}

function apply(target) {
  const listPath = join(target, LIST);
  if (!existsSync(listPath)) {
    console.log(`No ${listPath}: a cold target, nothing to apply`);
    return;
  }
  const before = parseList(readFileSync(listPath, "utf8"));
  const now = parseList(stagedList());
  const moved = new Set();
  for (const path of now.keys()) if (!before.has(path)) for (const directory of ancestors(path)) moved.add(directory);
  for (const path of before.keys()) if (!now.has(path)) for (const directory of ancestors(path)) moved.add(directory);
  let unchanged = 0;
  const changed = [];
  const directories = new Set();
  for (const [path, blob] of now) {
    for (const directory of ancestors(path)) directories.add(directory);
    if (before.get(path) !== blob) {
      changed.push(path);
      continue;
    }
    // A sparse or partial checkout can leave a tracked file out.
    if (!existsSync(path)) continue;
    lutimesSync(path, UNCHANGED_TIME, UNCHANGED_TIME);
    unchanged += 1;
  }
  let settled = 0;
  for (const directory of directories) {
    if (moved.has(directory) || !existsSync(directory)) continue;
    lutimesSync(directory, UNCHANGED_TIME, UNCHANGED_TIME);
    settled += 1;
  }
  const removed = [...before.keys()].filter((path) => !now.has(path)).length;
  console.log(`${unchanged} files and ${settled} directories unchanged since the target's build; ${changed.length} changed or added, ${removed} removed`);
  for (const path of changed.slice(0, 50)) console.log(`  changed: ${path}`);
  if (changed.length > 50) console.log(`  ... and ${changed.length - 50} more`);
}

const [command, target] = process.argv.slice(2);
if ((command !== "record" && command !== "apply") || target === undefined || process.argv.length > 4) {
  console.error("Usage: node scripts/build/target-sources.mjs record|apply <target-dir>");
  process.exit(2);
}
if (command === "record") record(target);
else apply(target);
