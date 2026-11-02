#!/usr/bin/env node
// Makes the GitHub Release of each released version on its release tag, with that version's section of the package's
// CHANGELOG.md as its notes, so release notes are written once, in the changelog (docs/release.md, "GitHub Releases").
// Core's Release, on `v<version>`, is a pre-release on `next` and `beta` and the repository's latest release on
// `latest`; a pack's, on `<name>@<version>`, is never the latest release, and is a pre-release only when published
// under `next` (held). A Release that exists already has its notes rewritten, so any run can be run again.
//
// Release mode names what a release run published, as dist-tags-check.mjs takes it, and reads the notes from the
// checkout, which must be the released commit: release.yml runs it for core, release-packs.yml for the packs.
// Backfill mode makes the Release of every release tag in this checkout that has none (the oldest first), with the
// notes at that tag's commit; fetch the tags first (`git fetch --tags`). Every backfilled pack Release is taken as
// published under `latest`, and only the highest core version on `latest` is marked latest. `--dry-run` prints each
// Release it would make, with its notes, and changes nothing on GitHub. gh needs write access to the repository.
//
//   node scripts/release/github-release.mjs [--dry-run] <name>@<version>[:<dist-tag>] ...   (or $RELEASED, the same, space-separated)
//   node scripts/release/github-release.mjs --backfill [--dry-run]
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import semver from "semver";
import { changelogSection, CORE_NAME, coreTag, NO_CHANGELOG_ENTRIES } from "./core.mjs";
import { parseExpectation } from "./dist-tags.mjs";
import { releaseTag, ROOT } from "./packs.mjs";

const PACK_PREFIX = "@neralabs/rerail-pack-";
const CORE_RELEASE_TAG = /^v(\d+\.\d+\.\d+(?:-(beta|next)\.\d+)?)$/u;
const PACK_RELEASE_TAG = /^(@neralabs\/rerail-pack-[a-z0-9-]+)@(\d+\.\d+\.\d+\S*)$/u;

/** The changelog holding `name`'s release notes: core's root CHANGELOG.md, or a pack's packs/<id>/CHANGELOG.md. */
export function changelogPath(name) {
  if (name === CORE_NAME) return "CHANGELOG.md";
  if (name.startsWith(PACK_PREFIX)) return `packs/${name.slice(PACK_PREFIX.length)}/CHANGELOG.md`;
  throw new Error(`${name} is neither core nor a rule pack`);
}

/**
 * The GitHub Release of `name`'s `version`, published under npm dist-tag `distTag`, given the text of its changelog:
 * `{ tag, title, notes, prerelease, latest }`, its notes the version's changelog section.
 */
export function githubRelease({ name, version, distTag }, changelog) {
  const core = name === CORE_NAME;
  const tag = core ? coreTag(version) : releaseTag(name, version);
  const section = changelogSection(changelog, version);
  return { tag, title: tag, notes: section === "" ? NO_CHANGELOG_ENTRIES : section, prerelease: distTag !== "latest", latest: core && distTag === "latest" };
}

/** gh's arguments that make `release` on GitHub with its notes from stdin; when it `exists`, that rewrite its notes. */
export function ghReleaseArgs(release, exists) {
  if (exists) return ["release", "edit", release.tag, "--notes-file", "-"];
  return ["release", "create", release.tag, "--verify-tag", "--title", release.title, "--notes-file", "-", `--prerelease=${release.prerelease}`, `--latest=${release.latest}`];
}

/**
 * The released version a git tag marks, `{ name, version, distTag }`, or null for a tag that marks none. Core's channel
 * is the tag's, as release.yml reads it; a pack's is taken to be `latest`, since a held publish leaves no trace in git.
 */
export function releasedByTag(tag) {
  const core = CORE_RELEASE_TAG.exec(tag);
  if (core !== null) return { name: CORE_NAME, version: core[1], distTag: core[2] ?? "latest" };
  const pack = PACK_RELEASE_TAG.exec(tag);
  return pack === null ? null : { name: pack[1], version: pack[2], distTag: "latest" };
}

/**
 * The GitHub Releases missing for `tags`, in their order, given the `existing` tags that have one and
 * `readAt(tag, path)`, a file's text at a tag (null when it has none). Only the highest core version released to
 * `latest` is marked latest, and only when its Release is among them.
 */
export function backfillPlan(tags, existing, readAt) {
  const tagged = tags.map((tag) => ({ tag, released: releasedByTag(tag) })).filter(({ released }) => released !== null);
  const [newest] = tagged
    .filter(({ released }) => released.name === CORE_NAME && released.distTag === "latest")
    .map(({ released }) => released.version)
    .sort(semver.rcompare);
  return tagged
    .filter(({ tag }) => !existing.has(tag))
    .map(({ tag, released }) => {
      const release = githubRelease(released, readAt(tag, changelogPath(released.name)) ?? "");
      return { ...release, latest: release.latest && released.version === newest };
    });
}

const run = (command, args, input) => execFileSync(command, args, { cwd: ROOT, encoding: "utf8", input, maxBuffer: 1 << 28 });

/** The tags of every Release on GitHub (`GH_REPO`, else the checkout's remote). */
const existingReleases = () =>
  new Set(
    run("gh", ["api", "--paginate", "repos/{owner}/{repo}/releases", "--jq", ".[].tag_name"])
      .split("\n")
      .filter((tag) => tag !== ""),
  );

/** A file's text at tag `tag`, or null when the tag's commit has no such file. */
function readAt(tag, path) {
  const object = `refs/tags/${tag}:${path}`;
  try {
    execFileSync("git", ["cat-file", "-e", object], { cwd: ROOT, stdio: "ignore" });
  } catch {
    return null;
  }
  return run("git", ["show", object]);
}

/** Makes each Release on GitHub, or prints it with its notes for a dry run. */
function makeReleases(releases, existing, dryRun) {
  for (const release of releases) {
    const exists = existing.has(release.tag);
    const kind = `${release.prerelease ? "pre-release" : "release"}, ${release.latest ? "latest" : "not latest"}`;
    const what = exists ? `the notes of the GitHub Release ${release.tag}` : `the GitHub Release ${release.tag} (${kind})`;
    if (dryRun) {
      console.log(`Would ${exists ? "rewrite" : "create"} ${what}, titled "${release.title}", with these notes:\n\n${release.notes}\n`);
      continue;
    }
    run("gh", ghReleaseArgs(release, exists), release.notes);
    console.log(`${exists ? "Rewrote" : "Created"} ${what}`);
  }
}

function main(args) {
  const dryRun = args.includes("--dry-run");
  const backfill = args.includes("--backfill");
  const rest = args.filter((arg) => arg !== "--dry-run" && arg !== "--backfill");
  const existing = existingReleases();
  if (backfill) {
    if (rest.length > 0) throw new Error(`--backfill reads every release tag in the checkout; it takes no ${rest.join(" ")}`);
    const tags = run("git", ["for-each-ref", "--sort=creatordate", "--format=%(refname:lstrip=2)", "refs/tags"]).split("\n").filter((tag) => tag !== "");
    const releases = backfillPlan(tags, existing, readAt);
    makeReleases(releases, existing, dryRun);
    const had = tags.filter((tag) => releasedByTag(tag) !== null).length - releases.length;
    console.log(`${dryRun ? "Would create" : "Created"} ${releases.length} GitHub Release(s); ${had} release tag(s) already have one.`);
    return;
  }
  const named = rest.length > 0 ? rest : (process.env.RELEASED ?? "").split(/\s+/u).filter((entry) => entry !== "");
  if (named.length === 0) throw new Error("nothing released: name <name>@<version>[:<dist-tag>] or set RELEASED, or pass --backfill");
  const releases = named
    .map(parseExpectation)
    .map(({ name, version, tag }) => githubRelease({ name, version, distTag: tag }, readFileSync(join(ROOT, changelogPath(name)), "utf8")));
  makeReleases(releases, existing, dryRun);
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main(process.argv.slice(2));
