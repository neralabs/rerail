// What a release note may say (docs/release.md, "Release notes"): a core changelog entry (.changelog/) or a pack's
// changeset summary goes out as it is on the GitHub Release, to the people who upgrade. Nothing here needs
// node_modules: the Changesets job of release-packs.yml installs none.
import { basename } from "node:path";
import { TICKET } from "../check/check-internal-refs.mjs";

/** The most words a release note (a changelog entry, a changeset summary) may run to. */
export const RELEASE_NOTE_WORDS = 60;

/**
 * A commit hash: 7 to 40 hex digits with a digit and a letter among them, so a byte count (`10485760`) or a CVE id's
 * digits are not read as one; a short hash with no letter is rare enough to miss.
 */
export const COMMIT_HASH = /\b(?=[0-9a-f]*\d)(?=[0-9a-f]*[a-f])[0-9a-f]{7,40}\b/u;

/** A pull request number: `#` and two or more digits, ending the note or followed by punctuation or a space. */
const PULL_REQUEST = /(?:^|[\s(])#(\d{2,})(?=[\s).,;:]|$)/u;

/** Keep a Changelog 1.1.0's change types, in the order a release's notes list them. */
export const CHANGE_TYPES = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"];

/** The change type a release note's file name opens with (`fixed-dashboard-toasts.md`), or null. */
export function changeType(path) {
  const file = basename(path);
  return CHANGE_TYPES.find((type) => file.startsWith(`${type.toLowerCase()}-`) && file.length > `${type}-.md`.length) ?? null;
}

/** Why the release note at `path` can't go under a heading of a release's notes, or null: its name gives no type. `readme` says how to name it. */
export function changeTypeRefusal(path, readme) {
  if (changeType(path) !== null) return null;
  const prefixes = CHANGE_TYPES.map((type) => `${type.toLowerCase()}-`);
  return `names no change type: start its file name with ${prefixes.slice(0, -1).join(", ")} or ${prefixes.at(-1)} (${readme})`;
}

/** `entries` (`{ type, entry }`) as a release's sections, one `### <type>` each, in CHANGE_TYPES' order, leaving out the types with none. */
export function changeTypeSections(entries) {
  return CHANGE_TYPES.map((type) => [type, entries.filter((entry) => entry.type === type)])
    .filter(([, group]) => group.length > 0)
    .map(([type, group]) => `### ${type}\n\n${group.map(({ entry }) => entry).join("\n")}`);
}

const wordCount = (text) => text.split(/\s+/u).filter((word) => word !== "").length;

/**
 * The CHANGELOG.md entry a release note's text becomes (a pack's changeset summary, a core entry under .changelog/):
 * `- <first line>`, its later lines indented under it, so its paragraphs stay under the bullet.
 */
export function releaseNoteEntry(text) {
  const [first, ...rest] = text.trim().split("\n");
  return [`- ${first}`, ...rest.map((line) => (line.trim() === "" ? "" : `  ${line}`))].join("\n");
}

/**
 * Why `note`, a release note (a changelog entry or a changeset summary, `- ` and all), is not written for the person
 * upgrading, or null: it runs past `words` words (RELEASE_NOTE_WORDS; null for no limit, a lead paragraph's), or
 * names an internal ticket (TICKET, as check-internal-refs.mjs reads it), a pull request or a commit, which say nothing
 * to them.
 */
export function releaseNoteRefusal(note, words = RELEASE_NOTE_WORDS) {
  const text = note.replace(/^- /u, "");
  const count = wordCount(text);
  if (words !== null && count > words) return `runs to ${count} words; a release note says what changed in a sentence or two (at most ${words} words)`;
  const ticket = TICKET.exec(text);
  if (ticket !== null) return `names the ticket ${ticket[0]}, which says nothing to the person upgrading`;
  const pullRequest = PULL_REQUEST.exec(text);
  if (pullRequest !== null) return `names pull request #${pullRequest[1]}, which says nothing to the person upgrading`;
  const commit = COMMIT_HASH.exec(text);
  if (commit !== null) return `names the commit ${commit[0]}, which says nothing to the person upgrading`;
  return null;
}
