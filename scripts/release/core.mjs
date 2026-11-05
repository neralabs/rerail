// Shared by the core release scripts in this directory (docs/distribution-model.md, "Release channels"). The core
// package @neralabs/rerail is versioned by the Release pull request, not by changesets: when anything core ships has
// changed since its last release (core-paths.mjs), the pull request bumps its patch version and writes the root
// CHANGELOG.md's section for it from the entries pending in .changelog/, by change type, below the lead written under
// the unreleased heading. Each released core version is marked by the git tag `v<version>`, which release.yml publishes.
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import semver from "semver";
import { CHANGE_TYPES, changeType, changeTypeRefusal, changeTypeSections, releaseNoteEntry } from "./release-notes.mjs";

export const CORE_NAME = "@neralabs/rerail";

/** The git tag that releases core's `version`: release.yml publishes on it. */
export const coreTag = (version) => `v${version}`;

/**
 * Where core's changelog entries wait for the Release pull request, one file each, so pull requests that add one never
 * edit the same lines (.changelog/README.md).
 */
export const CHANGELOG_ENTRIES = ".changelog";

export { CHANGE_TYPES, changeType as changelogEntryType };

/** Why the pending entry at `path` can't go under a heading of a release's notes, or null: its name gives no type. */
export const changelogEntryTypeRefusal = (path) => changeTypeRefusal(path, `${CHANGELOG_ENTRIES}/README.md`);

/**
 * Core's pending changelog entries in the checkout at `root`: every `.md` file in CHANGELOG_ENTRIES but its README, in
 * file-name order, as `{ path, type, text, entry }`, `type` one of CHANGE_TYPES (null when the name gives none), `text`
 * the file as written and `entry` the CHANGELOG.md entry it becomes (releaseNoteEntry).
 */
export function readChangelogEntries(root) {
  const directory = join(root, CHANGELOG_ENTRIES);
  if (!existsSync(directory)) return [];
  return readdirSync(directory)
    .filter((file) => file.endsWith(".md") && file !== "README.md")
    .sort()
    .map((file) => `${CHANGELOG_ENTRIES}/${file}`)
    .map((path) => {
      const text = readFileSync(join(root, path), "utf8");
      return { path, type: changeType(path), text, entry: releaseNoteEntry(text) };
    });
}

/** The first `## ` heading of a changelog and where it is, or null. */
function firstSection(text) {
  const match = /^## (.*)$/mu.exec(text);
  return match === null ? null : { heading: match[1].trim(), index: match.index, length: match[0].length };
}

/** The version an unreleased heading (`Unreleased`, `<version> (unreleased)`) names, `""` for none; null if released. */
function unreleasedVersion(heading) {
  if (/^\[?unreleased\]?$/iu.test(heading)) return "";
  const match = /^(\S+) \(unreleased\)$/iu.exec(heading);
  return match === null ? null : match[1];
}

/**
 * The version core's next release takes after `current`: a patch bump (`0.1.0-next.4` -> `0.1.0`, `0.1.0` -> `0.1.1`),
 * unless the changelog's unreleased heading names a later version (`## 0.2.0 (unreleased)`), which is how a release
 * other than a patch is asked for. Throws on a named version that is not valid or not later than `current`.
 */
export function nextCoreVersion(current, changelog) {
  const patch = semver.inc(current, "patch");
  const section = firstSection(changelog);
  const named = section === null ? null : unreleasedVersion(section.heading);
  if (named === null || named === "") return patch;
  if (semver.valid(named) !== named) throw new Error(`CHANGELOG.md's unreleased heading names ${named}, which is not a version (write it without a leading v)`);
  if (!semver.gt(named, current)) throw new Error(`CHANGELOG.md's unreleased heading names ${named}, which is not later than core's ${current}`);
  return named;
}

/** The notes of a release that had none: its changelog section, and its GitHub Release (github-release.mjs). */
export const NO_CHANGELOG_ENTRIES = "No changelog entries were written for this release.";

/**
 * The root CHANGELOG.md released as `version` on `date` (YYYY-MM-DD), with `entries` (`{ type, entry }`, in order) the
 * pending ones from CHANGELOG_ENTRIES: the unreleased section, if any, becomes `## <version> (<date>)`, its lead (and
 * any entry written under it) followed by `entries` under a `### <type>` heading each, in CHANGE_TYPES' order, leaving
 * out the types with none. Without an unreleased section the dated section is added above the last release; with
 * neither a lead nor an entry it says so. `notes` is whether the release had notes. Throws on an entry with no type,
 * which would otherwise drop out of the notes.
 */
export function releaseChangelog(text, version, date, entries) {
  const untyped = entries.find(({ type }) => !CHANGE_TYPES.includes(type));
  if (untyped !== undefined) throw new Error(`a changelog entry has no change type: ${untyped.entry}`);
  const section = firstSection(text);
  const unreleased = section !== null && unreleasedVersion(section.heading) !== null;
  const before = section === null ? `${text.replace(/\n*$/u, "")}\n\n` : text.slice(0, section.index);
  const rest = section === null ? "" : text.slice(section.index + (unreleased ? section.length : 0));
  const next = unreleased ? /^## /mu.exec(rest) : null;
  const written = unreleased ? (next === null ? rest : rest.slice(0, next.index)).trim() : "";
  const after = unreleased ? (next === null ? "" : rest.slice(next.index)) : rest;
  const parts = [written, ...changeTypeSections(entries)].filter((part) => part !== "");
  const body = parts.length === 0 ? NO_CHANGELOG_ENTRIES : parts.join("\n\n");
  return { text: `${before}## ${version} (${date})\n\n${body}\n${after === "" ? "" : `\n${after}`}`, notes: parts.length > 0 };
}

/** The body of the changelog section headed `## <version>` (to the next `## `), trimmed; `""` when there is none. */
export function changelogSection(text, version) {
  const lines = text.split("\n");
  const start = lines.findIndex((line) => line === `## ${version}` || line.startsWith(`## ${version} (`));
  if (start === -1) return "";
  const end = lines.findIndex((line, index) => index > start && line.startsWith("## "));
  return lines.slice(start + 1, end === -1 ? undefined : end).join("\n").trim();
}

/**
 * The first section of a changelog: `{ heading, released, body }`, `released` false for an unreleased heading
 * (`Unreleased`, `<version> (unreleased)`), the body to the next `## ` heading, trimmed; null with no section.
 */
export function firstChangelogSection(text) {
  const section = firstSection(text);
  if (section === null) return null;
  const rest = text.slice(section.index + section.length);
  const next = /^## /mu.exec(rest);
  return { heading: section.heading, released: unreleasedVersion(section.heading) === null, body: (next === null ? rest : rest.slice(0, next.index)).trim() };
}

/**
 * A changelog section's lead: the prose before its first entry or `### ` heading, trimmed, `""` when it opens with one.
 * It is the release's pitch to the people who upgrade (docs/release.md, "Release notes").
 */
export function changelogLead(section) {
  const lines = section.split("\n");
  const end = lines.findIndex((line) => line.startsWith("- ") || line.startsWith("### "));
  return lines.slice(0, end === -1 ? undefined : end).join("\n").trim();
}

/**
 * A changelog section's entries: each `- ` line with the indented lines under it, as one string each, blank lines
 * between an entry's paragraphs kept, so an entry reads as it is written and is found again in an earlier text.
 */
export function changelogEntries(section) {
  const entries = [];
  const lines = section.split("\n");
  let blanks = 0;
  for (const line of lines) {
    if (line.startsWith("- ")) {
      entries.push(line);
      blanks = 0;
    } else if (line.trim() === "") {
      blanks += 1;
    } else if (entries.length > 0 && /^\s+\S/u.test(line)) {
      entries[entries.length - 1] += `${"\n".repeat(blanks + 1)}${line}`;
      blanks = 0;
    } else {
      blanks = 0;
    }
  }
  return entries;
}

/**
 * Why a release's changelog section, as firstChangelogSection reads it, has no lead for the people who upgrade, or
 * null: a released (dated) section that opens with an entry. An unreleased section is not asked for one yet, and a
 * section with no entries (the release had no notes) is not asked for one at all.
 */
export function changelogLeadRefusal(section) {
  if (!section.released || changelogLead(section.body) !== "" || changelogEntries(section.body).length === 0) return null;
  return `CHANGELOG.md's ${section.heading} section opens with an entry: lead it with a paragraph saying what this release gives the people who upgrade, written under \`## Unreleased\` on main (docs/release.md, "Release notes")`;
}

/** The entries of `after`'s first section that `before` (a changelog's earlier text, or null) does not hold. */
export function newChangelogEntries(before, after) {
  const section = firstChangelogSection(after);
  if (section === null) return [];
  return changelogEntries(section.body).filter((entry) => before === null || !before.includes(entry));
}

/**
 * The entries a pull request writes into `after`'s first section (CHANGELOG.md) by hand rather than in
 * CHANGELOG_ENTRIES: every one of newChangelogEntries, under `## Unreleased` or under a dated heading (main's top
 * section right after a release), except on the Release pull request, whose first section is a dated heading the
 * base's (`before`, or null) does not open with and holds the entries it moved out of CHANGELOG_ENTRIES.
 */
export function handWrittenChangelogEntries(before, after) {
  const section = firstChangelogSection(after);
  if (section === null) return [];
  if (section.released && firstChangelogSection(before ?? "")?.heading !== section.heading) return [];
  return newChangelogEntries(before, after);
}

/**
 * The whole entries of `text`'s first section (CHANGELOG.md, or null) when it is unreleased, where entries waited
 * before CHANGELOG_ENTRIES held them; `[]` when that section is dated or there is none.
 */
export function unreleasedChangelogEntries(text) {
  const section = text === null ? null : firstChangelogSection(text);
  return section === null || section.released ? [] : changelogEntries(section.body);
}

/**
 * Why `text`, a file in CHANGELOG_ENTRIES, is no entry the Release pull request can add, or null: it is empty, or it
 * opens with a list marker, which releaseNoteEntry would nest under the bullet it adds (`- - …`).
 */
export function changelogEntryFileRefusal(text) {
  const trimmed = text.trim();
  if (trimmed === "") return "is empty: write what changed for the person upgrading";
  const marker = /^([-*+])\s/u.exec(trimmed);
  if (marker === null) return null;
  return `starts with \`${marker[1]} \`: write the entry's text alone, which the Release pull request adds to the notes under a bullet of its own (${CHANGELOG_ENTRIES}/README.md)`;
}

/** `package.json`'s text with its own `version` set to `version`, and nothing else changed. */
export function setPackageVersion(text, version) {
  const updated = text.replace(/^( {2}"version": ")[^"]*(")/mu, `$1${version}$2`);
  if (updated === text && JSON.parse(text).version !== version) throw new Error("package.json has no top-level version to set");
  return updated;
}

/** `package-lock.json` with the root package's recorded version set to `version`, and nothing else changed. */
export function setLockVersion(lock, version) {
  if (lock.packages?.[""] === undefined) throw new Error('package-lock.json has no root package (""); run npm install');
  return { ...lock, version, packages: { ...lock.packages, "": { ...lock.packages[""], version } } };
}

/**
 * The files of the installer plugins that coding agents' plugin directories list (`plugins/<agent>`) that pin core's
 * version: each manifest's `version`, and the `npx @neralabs/rerail@<version>` each `install` skill runs and each
 * README shows.
 */
export const PLUGIN_PINS = ["claude", "codex", "cursor"].flatMap((agent) => [
  { claude: "plugins/claude/.claude-plugin/plugin.json", codex: "plugins/codex/plugin.json", cursor: "plugins/cursor/.cursor-plugin/plugin.json" }[agent],
  `plugins/${agent}/skills/install/SKILL.md`,
  `plugins/${agent}/README.md`,
]);

/** The text of one of PLUGIN_PINS with its manifest `version` and every `@neralabs/rerail@<version>` set to `version`. */
export function setPluginVersion(text, version) {
  return text.replace(/^( {2}"version": ")[^"]*(")/mu, `$1${version}$2`).replace(/@neralabs\/rerail@\d[^\s`]*/gu, `@neralabs/rerail@${version}`);
}

/**
 * Whether core `version` may be tagged for release, given the `v*` tags that exist: it must not be tagged yet and must
 * be later than every released version, so a version moved backwards by mistake is never published. Returns the
 * reason it may not, or null.
 */
export function coreTagRefusal(version, tags) {
  if (semver.valid(version) !== version) return `package.json's version ${version} is not a version`;
  const released = tags.filter((tag) => tag.startsWith("v")).map((tag) => tag.slice(1)).filter((tagged) => semver.valid(tagged) !== null);
  if (released.includes(version)) return `${coreTag(version)} already exists`;
  const later = released.filter((tagged) => semver.gte(tagged, version)).sort(semver.rcompare);
  return later.length === 0 ? null : `${version} is not later than the released ${later[0]}`;
}
