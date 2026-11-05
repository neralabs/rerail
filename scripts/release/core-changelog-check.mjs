#!/usr/bin/env node
// Core's release notes are written for the person upgrading (docs/release.md, "Release notes"). On a pull request,
// every entry pending in .changelog/ (one file each) names its change type, and each it adds or changes stays at a
// sentence or two, names no ticket, pull request or commit, and is the entry's text alone, without the bullet the
// Release pull request adds; and no entry is written into CHANGELOG.md's first section, unreleased or dated, where every pull request would edit the same lines
// (or a published release's notes). On the Release pull request, whose first section is a dated heading new against
// the base and holds the entries it moved out of .changelog/ (checked when they were added), the section leads with a
// paragraph for the people who upgrade, written under `## Unreleased` on main. ci.yml's lint job runs it.
//
//   node scripts/release/core-changelog-check.mjs BASE   (BASE: the base branch's tip, `origin/main`, fetched)
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { CHANGELOG_ENTRIES, changelogEntryFileRefusal, changelogEntryTypeRefusal, changelogLead, changelogLeadRefusal, firstChangelogSection, handWrittenChangelogEntries, readChangelogEntries, unreleasedChangelogEntries } from "./core.mjs";
import { releaseNoteRefusal } from "./release-notes.mjs";
import { ROOT } from "./packs.mjs";

const base = process.argv[2];
if (base === undefined) throw new Error("usage: core-changelog-check.mjs BASE");

/** Repository file `path` as the base holds it, or null when it holds none. */
function atBase(path) {
  try {
    return execFileSync("git", ["show", `${base}:${path}`], { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch {
    return null;
  }
}

/** The text of every entry the base holds in .changelog/, whatever its file is named there. */
function baseEntryTexts() {
  let listed = "";
  try {
    listed = execFileSync("git", ["ls-tree", "--name-only", base, `${CHANGELOG_ENTRIES}/`], { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  } catch {
    // A base that holds no such folder holds no entries.
  }
  return new Set(listed.split("\n").filter((path) => path.endsWith(".md") && path !== `${CHANGELOG_ENTRIES}/README.md`).map(atBase));
}

const quote = (entry) => `"${entry.replace(/\s+/gu, " ").slice(0, 60)}…"`;

const after = readFileSync(join(ROOT, "CHANGELOG.md"), "utf8");
const before = atBase("CHANGELOG.md");
if (before === null) console.log(`${base} has no CHANGELOG.md: every entry counts as new`);

/** Each failure, `{ file, title, text }`, as a GitHub annotation and a line. */
const failures = [];
const section = firstChangelogSection(after);
if (section !== null) {
  const lead = changelogLead(section.body);
  const missingLead = changelogLeadRefusal(section);
  if (missingLead !== null) failures.push({ file: "CHANGELOG.md", title: "Release notes need a lead", text: missingLead });
  const leadRefusal = lead === "" ? null : releaseNoteRefusal(lead, null);
  if (leadRefusal !== null) failures.push({ file: "CHANGELOG.md", title: "Release notes lead", text: `CHANGELOG.md's ${section.heading} lead ${leadRefusal}` });
  for (const entry of handWrittenChangelogEntries(before, after)) {
    const refusal = `is written in CHANGELOG.md's ${section.heading} section: write it in a file of its own in ${CHANGELOG_ENTRIES}/, which the Release pull request adds to the release's notes (${CHANGELOG_ENTRIES}/README.md)`;
    failures.push({ file: "CHANGELOG.md", title: "Changelog entry", text: `the entry ${quote(entry)} ${refusal}` });
  }
}

// An entry the base holds already, in a file under any name (renaming one to give it a type changes no word of it) or
// as a whole entry of CHANGELOG.md's unreleased section (moved out of it), is not this pull request's. Its type is
// checked all the same: the Release pull request can't place an entry without one.
const moved = unreleasedChangelogEntries(before);
const held = baseEntryTexts();
for (const { path, text, entry } of readChangelogEntries(ROOT)) {
  const typeRefusal = changelogEntryTypeRefusal(path);
  if (typeRefusal !== null) failures.push({ file: path, title: "Changelog entry", text: `${path} ${typeRefusal}` });
  if (held.has(text)) continue;
  const fileRefusal = changelogEntryFileRefusal(text);
  if (fileRefusal !== null) {
    failures.push({ file: path, title: "Changelog entry", text: `${path} ${fileRefusal}` });
    continue;
  }
  if (moved.includes(entry)) continue;
  const refusal = releaseNoteRefusal(entry);
  if (refusal !== null) failures.push({ file: path, title: "Changelog entry", text: `${path} ${quote(entry)} ${refusal}` });
}

for (const failure of failures) console.log(`::error file=${failure.file},title=${failure.title}::${failure.text}`);
console.log(failures.length === 0 ? "Release notes: every entry this pull request adds is written for the person upgrading." : `Release notes: ${failures.length} problem(s).`);
process.exit(failures.length === 0 ? 0 : 1);
