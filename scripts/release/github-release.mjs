#!/usr/bin/env node
// Makes the GitHub Release of each released version on its release tag, with that version's section of the package's
// CHANGELOG.md as its notes, so release notes are written once, in the changelog (docs/release.md, "GitHub Releases").
// Core's Release, on `v<version>`, is a pre-release on `next` and `beta` and the repository's latest release on
// `latest`; a pack's, on `<name>@<version>`, is never the latest release, and is a pre-release while it is on `next`.
// A release publishes to `next`, so its Releases start as pre-releases. A Release that exists already has its notes,
// pre-release flag and latest flag rewritten, so any run can be run again, and a promotion or a put-back (release.yml's
// promote job or `latest` input, release-packs.yml's promote) rewrites the flags for `latest`. Promoted to `latest`, a
// version's notes are everything since the last version stable users had (previousStable, sinceStableSection): its own
// entries and those of each release candidate on `next` it passed over.
//
// Release mode names what a release run published, as dist-tags-check.mjs takes it, and reads the notes from the
// checkout's changelog: the released commit when a release publishes, and main when it is promoted or put back, whose
// changelog keeps every released version's section and any note corrected while the release was tested. A version
// whose changelog has no section of its own gets a Release saying no entries were written, as the Release pull request
// says. The pre-release flag is the record of which versions reached `latest`: previousStable reads it.
// Backfill mode makes the Release of every release tag on the remote that has none (the oldest first), with the notes
// at that tag's commit; fetch the tags first (`git fetch --tags --prune --prune-tags`). It skips, and names, a local
// tag the remote does not have, a tag whose local commit is not the remote's (fetch it again with `--force`), and a
// tag whose changelog has no section for its version, so a backfilled Release always has notes. A backfilled plain
// version is taken as on `latest` unless it is above its package's `latest` on npm, which leaves it on `next` and its
// Release a pre-release; only the highest core version on `latest` is marked latest.
// `--dry-run` prints each Release it would make, with its notes, and changes nothing on GitHub. gh needs write access
// to the repository.
//
//   node scripts/release/github-release.mjs [--dry-run] <name>@<version>[:<dist-tag>] ...   (or $RELEASED, the same, space-separated)
//   node scripts/release/github-release.mjs --backfill [--dry-run]
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import semver from "semver";
import { changelogEntries, changelogLead, changelogSection, CORE_NAME, coreTag, NO_CHANGELOG_ENTRIES } from "./core.mjs";
import { NEXT_TAG, parseExpectation, readDistTagsOrNone } from "./dist-tags.mjs";
import { releaseTag, ROOT } from "./packs.mjs";
import { CHANGE_TYPES, changeTypeSections, COMMIT_HASH } from "./release-notes.mjs";

const PACK_PREFIX = "@neralabs/rerail-pack-";
const CORE_RELEASE_TAG = /^v(\d+\.\d+\.\d+(?:-(beta|next)\.\d+)?)$/u;
const PACK_RELEASE_TAG = /^(@neralabs\/rerail-pack-[a-z0-9-]+)@(\d+\.\d+\.\d+\S*)$/u;
/** The commit hash the default changeset generator led an entry with: `- <hash>: `. */
const HASH_LED_ENTRY = new RegExp(`^(\\s*- )(?:${COMMIT_HASH.source}): `, "u");
/** The remote the backfill takes the release tags from. */
const REMOTE = "origin";
/** Where docs/ is published (`SITE` in scripts/build/docs-site.mjs), and the repository other links point into. */
const DOCS_SITE = "https://neralabs.github.io/rerail";
const REPOSITORY = "https://github.com/neralabs/rerail";
/** The most characters GitHub takes as a Release's text; past it the call fails. */
export const RELEASE_NOTES_LIMIT = 125_000;

/** The changelog holding `name`'s release notes: core's root CHANGELOG.md, or a pack's packs/<id>/CHANGELOG.md. */
export function changelogPath(name) {
  if (name === CORE_NAME) return "CHANGELOG.md";
  if (name.startsWith(PACK_PREFIX)) return `packs/${name.slice(PACK_PREFIX.length)}/CHANGELOG.md`;
  throw new Error(`${name} is neither core nor a rule pack`);
}

/**
 * A pack's changelog section as its Release notes. A section the Release pull request wrote (pack-changelog.mjs) goes
 * as it is. One from before it, which a backfill reads, loses the one `### <Bump> Changes` heading changesets wrote over
 * every section of a pack below 1.0.0 (kept when a section has two or more) and the commit hash its default generator
 * led each entry with.
 */
export function packReleaseNotes(section) {
  const lines = section.split("\n").map((line) => line.replace(HASH_LED_ENTRY, "$1"));
  const headings = lines.filter((line) => /^### \w+ Changes$/u.test(line));
  if (headings.length !== 1) return lines.join("\n").trim();
  const at = lines.indexOf(headings[0]);
  return lines.filter((line, index) => index !== at && (index !== at + 1 || line !== "")).join("\n").trim();
}

/**
 * `notes` with each relative Markdown link, written relative to `base` (the changelog's folder), made absolute: GitHub
 * resolves a Release's relative links against the release page, where they lead nowhere. A page of docs/ goes to its
 * page on the docs website, as docs-site.mjs names it (`x.md` and `x.html` become `x.html`, `README.md` `index.html`);
 * any other path to the file on GitHub at `tag`. Absolute URLs and in-page anchors are left alone.
 */
export function absoluteLinks(notes, base, tag) {
  return notes.replace(/\]\(([^)\s]+)\)/gu, (link, target) => {
    if (/^(?:[a-z][a-z0-9+.-]*:|#|\/)/iu.test(target)) return link;
    const hash = target.indexOf("#");
    const anchor = hash === -1 ? "" : target.slice(hash);
    const file = posix.normalize(posix.join(base, hash === -1 ? target : target.slice(0, hash)));
    const page = /^docs\/([^/]+)\.(?:md|html)$/u.exec(file)?.[1];
    if (page !== undefined) return `](${DOCS_SITE}/${page === "README" ? "index" : page}.html${anchor})`;
    return `](${REPOSITORY}/blob/${tag}/${file}${anchor})`;
  });
}

/**
 * `notes` as a Release can hold them. Past RELEASE_NOTES_LIMIT, which a pack's first release can reach with one entry
 * per rule, they are the section's lead and a link to the changelog at `tag`: failing would only leave the version
 * without a Release.
 */
export function fittedNotes(notes, section, path, tag) {
  if (notes.length <= RELEASE_NOTES_LIMIT) return notes;
  return [changelogLead(section), `These notes are too long for a GitHub Release: [read them in ${path}](${REPOSITORY}/blob/${tag}/${path}).`].filter((part) => part !== "").join("\n\n");
}

/** The plain versions `changelog` has a section for (`## <version>` or `## <version> (<date>)`), newest first. */
function changelogVersions(changelog) {
  const versions = [...changelog.matchAll(/^## (\d+\.\d+\.\d+)(?: \(|$)/gmu)].map((match) => match[1]);
  return versions.sort(semver.rcompare);
}

/** A section's entries, each `{ type, entry }`: its `### <type>` heading's change type, null for one under none. */
function typedEntries(section) {
  const chunks = section.split(/^(?=### )/mu);
  return chunks.flatMap((chunk) => {
    const heading = /^### (\w+)/u.exec(chunk)?.[1];
    const type = CHANGE_TYPES.includes(heading) ? heading : null;
    return changelogEntries(chunk).map((entry) => ({ type, entry }));
  });
}

/** `versions` as a sentence lists them, oldest first: `0.2.0`, `0.2.0 and 0.2.1`, `0.2.0, 0.2.1 and 0.2.2`. */
const listed = (versions) => (versions.length === 1 ? versions[0] : `${versions.slice(0, -1).join(", ")} and ${versions.at(-1)}`);

/**
 * The notes of `version` promoted to `latest` over `since`, the last version stable users had: what changed for them.
 * With no version between them, `version`'s own section (`read` as the Release reads it). Otherwise its lead (none when
 * the section only says no entries were written), a line naming the release candidates it brings that never reached
 * `latest`, and the entries of every version after `since` up to `version`, newest first: those under no type heading,
 * as a section lists them under its lead, then the rest grouped by change type (changeTypeSections). Each version
 * keeps its own section in the changelog: only the Release brings them together.
 */
export function sinceStableSection(changelog, version, since, read = (section) => section) {
  const own = read(changelogSection(changelog, version));
  const skipped = changelogVersions(changelog).filter((candidate) => semver.gt(candidate, since) && semver.lt(candidate, version));
  if (skipped.length === 0) return own;
  const entries = [version, ...skipped].flatMap((each) => typedEntries(read(changelogSection(changelog, each))));
  const lead = own === NO_CHANGELOG_ENTRIES ? "" : changelogLead(own);
  const candidates = `${skipped.length === 1 ? "was a release candidate" : "were release candidates"} on \`next\` and never reached \`latest\``;
  const untyped = entries.filter(({ type }) => type === null).map(({ entry }) => entry);
  return [lead, `This release also brings ${listed([...skipped].reverse())}, which ${candidates}.`, untyped.join("\n"), ...changeTypeSections(entries.filter(({ type }) => type !== null))]
    .filter((part) => part !== "")
    .join("\n\n");
}

/**
 * The changelog text a Release of `name`'s `version` under `distTag` is written from, before githubRelease makes its
 * links absolute: everything since `since` once promoted (sinceStableSection), else the version's own section, a
 * pack's as packReleaseNotes reads it.
 */
function releaseSection({ name, version, distTag }, changelog, since) {
  const read = name === CORE_NAME ? (section) => section : packReleaseNotes;
  if (since !== null && distTag === "latest") return sinceStableSection(changelog, version, since, read);
  return read(changelogSection(changelog, version));
}

/**
 * The GitHub Release of `name`'s `version`, published under npm dist-tag `distTag`, given the text of its changelog:
 * `{ tag, title, notes, prerelease, latest }`, its notes the version's changelog section (a pack's as
 * packReleaseNotes reads it) with its links made absolute (absoluteLinks) and fitted to a Release (fittedNotes), or that
 * none was written. Promoted to `latest` over `since`, the last version on `latest` (previousStable), its notes are
 * everything since that version (sinceStableSection), so a release candidate that never reached `latest` is not lost
 * on the people who update past it.
 */
export function githubRelease({ name, version, distTag }, changelog, since = null) {
  const core = name === CORE_NAME;
  const tag = core ? coreTag(version) : releaseTag(name, version);
  const section = releaseSection({ name, version, distTag }, changelog, since);
  const path = changelogPath(name);
  const notes = section === "" ? NO_CHANGELOG_ENTRIES : fittedNotes(absoluteLinks(section, posix.dirname(path), tag), section, path, tag);
  return { tag, title: tag, notes, prerelease: distTag !== "latest", latest: core && distTag === "latest" };
}

/**
 * The last version of `name` before `version` that stable users had: the newest plain version below it whose GitHub
 * Release (`releases`, `{ tag, prerelease }` each) is no pre-release, which it is once promoted. Null before any.
 */
export function previousStable(name, version, releases) {
  const stable = releases
    .filter(({ prerelease }) => !prerelease)
    .map(({ tag }) => releasedByTag(tag))
    .filter((released) => released !== null && released.name === name && semver.prerelease(released.version) === null && semver.lt(released.version, version))
    .map((released) => released.version);
  return stable.sort(semver.rcompare)[0] ?? null;
}

/**
 * gh's arguments that make `release` on GitHub with its notes from stdin; when it `exists`, that rewrite its notes,
 * pre-release flag and latest flag, so a run made again, or a put-back, sets the flags right.
 */
export function ghReleaseArgs(release, exists) {
  const flags = [`--prerelease=${release.prerelease}`, `--latest=${release.latest}`];
  if (exists) return ["release", "edit", release.tag, "--notes-file", "-", ...flags];
  return ["release", "create", release.tag, "--verify-tag", "--title", release.title, "--notes-file", "-", ...flags];
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
 * The tags `git ls-remote --tags` lists, as a Map from tag to the commit it marks: an annotated tag's peeled `^{}`
 * line gives its commit, a lightweight tag's line the commit itself.
 */
export function parseRemoteTags(text) {
  const commits = new Map();
  for (const line of text.split("\n")) {
    const match = /^([0-9a-f]+)\trefs\/tags\/(.+?)(\^\{\})?$/u.exec(line);
    if (match === null) continue;
    const [, commit, tag, peeled] = match;
    if (peeled !== undefined || !commits.has(tag)) commits.set(tag, commit);
  }
  return commits;
}

/**
 * The release tags a backfill may read, given the checkout's `local` tags (`{ tag, commit }`, in the order to make
 * them) and the `remote`'s (tag to commit, parseRemoteTags): `{ tags, localOnly, diverged, missing }`. A local release
 * tag the remote does not have is `localOnly`, one at a commit other than the remote's is `diverged` (its notes would be
 * read from the wrong commit), and a remote release tag the checkout does not have is `missing`. Tags marking no
 * release are left out of all four.
 */
export function backfillTags(local, remote) {
  const tags = [];
  const localOnly = [];
  const diverged = [];
  for (const { tag, commit } of local) {
    if (releasedByTag(tag) === null) continue;
    const remoteCommit = remote.get(tag);
    if (remoteCommit === undefined) localOnly.push(tag);
    else if (remoteCommit !== commit) diverged.push(tag);
    else tags.push(tag);
  }
  const known = new Set(local.map(({ tag }) => tag));
  const missing = [...remote.keys()].filter((tag) => releasedByTag(tag) !== null && !known.has(tag));
  return { tags, localOnly, diverged, missing };
}

/**
 * The channel a backfill takes `released` (releasedByTag's) to be on, given `latest`, the version its package's npm
 * `latest` dist-tag points at (undefined when it has none): a version releasedByTag takes for `latest` that is above it
 * is still on `next`, as a release run that failed before its Release leaves it. Its Release is then a pre-release, as
 * that run would have made it, so previousStable never takes it for a version stable users had.
 */
function backfilledChannel(released, latest) {
  const onNext = released.distTag === "latest" && (latest === undefined || semver.gt(released.version, latest));
  return onNext ? { ...released, distTag: NEXT_TAG } : released;
}

/**
 * The GitHub Releases a backfill makes for `tags`, in their order, given the `existing` tags that have one,
 * `readAt(tag, path)`, a file's text at a tag (null when it has none), and `onLatest`, each package's npm `latest`
 * version by name (backfilledChannel): `{ releases, noNotes }`, where `noNotes` lists the tags left without a Release
 * because their changelog has no section for their version. Only the highest core version on `latest` is marked
 * latest, and only when its Release is among them.
 */
export function backfillPlan(tags, existing, readAt, onLatest) {
  const tagged = tags
    .map((tag) => ({ tag, released: releasedByTag(tag) }))
    .filter(({ released }) => released !== null)
    .map(({ tag, released }) => ({ tag, released: backfilledChannel(released, onLatest.get(released.name)) }));
  const [newest] = tagged
    .filter(({ released }) => released.name === CORE_NAME && released.distTag === "latest")
    .map(({ released }) => released.version)
    .sort(semver.rcompare);
  const releases = [];
  const noNotes = [];
  for (const { tag, released } of tagged) {
    if (existing.has(tag)) continue;
    const changelog = readAt(tag, changelogPath(released.name)) ?? "";
    if (changelogSection(changelog, released.version) === "") {
      noNotes.push(tag);
      continue;
    }
    const release = githubRelease(released, changelog);
    releases.push({ ...release, latest: release.latest && released.version === newest });
  }
  return { releases, noNotes };
}

const run = (command, args, input) => execFileSync(command, args, { cwd: ROOT, encoding: "utf8", input, maxBuffer: 1 << 28 });

/** Every Release on GitHub (`GH_REPO`, else the checkout's remote), as `{ tag, prerelease }`. */
const releaseList = () =>
  run("gh", ["api", "--paginate", "repos/{owner}/{repo}/releases", "--jq", '.[] | "\\(.tag_name)\\t\\(.prerelease)"'])
    .split("\n")
    .filter((line) => line !== "")
    .map((line) => {
      const [tag, prerelease] = line.split("\t");
      return { tag, prerelease: prerelease === "true" };
    });

/** The checkout's tags, the oldest first, each with the commit it marks (an annotated tag's peeled). */
const localTags = () =>
  run("git", ["for-each-ref", "--sort=creatordate", "--format=%(refname:lstrip=2)\t%(objectname)\t%(*objectname)", "refs/tags"])
    .split("\n")
    .filter((line) => line !== "")
    .map((line) => {
      const [tag, object, peeled] = line.split("\t");
      return { tag, commit: peeled === "" ? object : peeled };
    });

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
    const what = exists ? `the notes and flags of the GitHub Release ${release.tag} (${kind})` : `the GitHub Release ${release.tag} (${kind})`;
    if (dryRun) {
      console.log(`Would ${exists ? "rewrite" : "create"} ${what}, titled "${release.title}", with these notes:\n\n${release.notes}\n`);
      continue;
    }
    run("gh", ghReleaseArgs(release, exists), release.notes);
    console.log(`${exists ? "Rewrote" : "Created"} ${what}`);
  }
}

/** The npm `latest` version of the package of each of `tags` that has no Release yet, by name (undefined for none). */
async function latestOnNpm(tags, existing) {
  const names = [...new Set(tags.filter((tag) => !existing.has(tag)).map((tag) => releasedByTag(tag).name))];
  return new Map(await Promise.all(names.map(async (name) => [name, (await readDistTagsOrNone(name)).latest])));
}

/** Makes the Release of every release tag on the remote that has none, skipping and naming what it cannot read. */
async function backfill(existing, dryRun) {
  const { tags, localOnly, diverged, missing } = backfillTags(localTags(), parseRemoteTags(run("git", ["ls-remote", "--tags", REMOTE])));
  if (missing.length > 0) throw new Error(`${REMOTE} has release tag(s) this checkout lacks: ${missing.join(", ")}; run git fetch --tags first`);
  if (localOnly.length > 0) console.log(`Skipped ${localOnly.length} local release tag(s) ${REMOTE} does not have (prune them with git fetch --prune --prune-tags): ${localOnly.join(", ")}`);
  if (diverged.length > 0) console.log(`Skipped ${diverged.length} release tag(s) not at ${REMOTE}'s commit in this checkout (fetch them again with git fetch --force --tags): ${diverged.join(", ")}`);
  const { releases, noNotes } = backfillPlan(tags, existing, readAt, await latestOnNpm(tags, existing));
  if (noNotes.length > 0) console.log(`Skipped ${noNotes.length} release tag(s) whose changelog has no section for the version: ${noNotes.join(", ")}`);
  makeReleases(releases, existing, dryRun);
  const had = tags.filter((tag) => existing.has(tag)).length;
  console.log(`${dryRun ? "Would create" : "Created"} ${releases.length} GitHub Release(s); ${had} release tag(s) already have one.`);
}

async function main(args) {
  const dryRun = args.includes("--dry-run");
  const backfillMode = args.includes("--backfill");
  const rest = args.filter((arg) => arg !== "--dry-run" && arg !== "--backfill");
  const releases = releaseList();
  const existing = new Set(releases.map(({ tag }) => tag));
  if (backfillMode) {
    if (rest.length > 0) throw new Error(`--backfill reads every release tag on ${REMOTE}; it takes no ${rest.join(" ")}`);
    await backfill(existing, dryRun);
    return;
  }
  const named = rest.length > 0 ? rest : (process.env.RELEASED ?? "").split(/\s+/u).filter((entry) => entry !== "");
  if (named.length === 0) throw new Error("nothing released: name <name>@<version>[:<dist-tag>] or set RELEASED, or pass --backfill");
  // Promoted to latest, a version's notes run from the last version stable users had.
  const made = named.map(parseExpectation).map(({ name, version, tag }) => {
    const since = tag === "latest" ? previousStable(name, version, releases) : null;
    return githubRelease({ name, version, distTag: tag }, readFileSync(join(ROOT, changelogPath(name)), "utf8"), since);
  });
  makeReleases(made, existing, dryRun);
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main(process.argv.slice(2));
