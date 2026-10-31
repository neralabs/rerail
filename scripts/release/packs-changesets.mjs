#!/usr/bin/env node
// A pull request that changes what a published pack publishes adds a changeset naming it (docs/pack-format.md,
// "Versioning and release"): without one, the change is never released. A private pack is never released, so it needs
// no changeset and no changeset may name it. Compares this checkout's HEAD with its merge base with BASE.
// The Release pull request writes versions and changelogs from the changesets themselves and is not checked.
//
//   node scripts/release/packs-changesets.mjs BASE   (BASE: the pull request's base commit, its history fetched)
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { listPacks, manifestChanged, missingChangesets, privateChangesetFailures, ROOT } from "./packs.mjs";

const base = process.argv[2];
if (base === undefined) throw new Error("usage: packs-changesets.mjs BASE");

const git = (args) => execFileSync("git", args, { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28 });
/** The paths `git diff -z <args>` names. `-z` goes first: after `--` it would be read as a path. */
const paths = (args) => git(["diff", "-z", ...args]).split("\0").filter((path) => path !== "");

const range = `${base}...HEAD`;
const mergeBase = git(["merge-base", base, "HEAD"]).trim();
/** A pack's package.json at the merge base; undefined where the pack is new. */
const before = (path) => {
  try {
    return git(["show", `${mergeBase}:${path}`]);
  } catch {
    return undefined;
  }
};
// A package.json whose only change is publishConfig changes nothing a release would ship (manifestChanged).
const changed = paths(["--name-only", "--no-renames", range, "--", "packs/"]).filter(
  (path) => !/^packs\/[^/]+\/package\.json$/u.test(path) || manifestChanged(before(path), readFileSync(join(ROOT, path), "utf8")),
);
const changesets = paths(["--name-only", "--no-renames", "--diff-filter=AM", range, "--", ".changeset/"])
  .filter((path) => path.endsWith(".md") && path !== ".changeset/README.md")
  .map((path) => ({ file: path, text: readFileSync(join(ROOT, path), "utf8") }));
const packs = listPacks().map((pack) => ({
  ...pack,
  files: JSON.parse(readFileSync(join(ROOT, "packs", pack.id, "package.json"), "utf8")).files ?? [],
}));

const failures = missingChangesets(changed, changesets, packs);
for (const failure of failures) console.log(`::error title=Changeset needed::${failure}. Run npx changeset and pick the pack (.changeset/README.md).`);
const privateFailures = privateChangesetFailures(changesets, packs);
for (const failure of privateFailures) console.log(`::error title=Changeset names a private pack::${failure}. Remove it from the changeset.`);
console.log(
  failures.length === 0
    ? `Changesets: every pack this pull request changes what it publishes of has one (${changesets.length} changeset(s) added or edited).`
    : `Changesets: ${failures.length} pack(s) change what they publish with no changeset.`,
);
process.exit(failures.length === 0 && privateFailures.length === 0 ? 0 : 1);
