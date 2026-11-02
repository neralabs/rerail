#!/usr/bin/env node
// Core's release notes are written for the person upgrading (docs/release.md, "Release notes"). On a pull request,
// each entry it adds or changes in .changelog/ (one file each) stays at a sentence or two and names no ticket, pull
// request or commit, and no entry is written into CHANGELOG.md's unreleased section, where every pull request would
// edit the same lines. On the Release pull request, whose first section is dated and holds the entries it moved out of
// .changelog/ (checked when they were added), the section leads with a paragraph for the people who upgrade, written
// under `## Unreleased` on main. ci.yml's lint job runs it.
//
//   node scripts/release/core-changelog-check.mjs BASE   (BASE: the base branch's tip, `origin/main`, fetched)
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { CHANGELOG_ENTRIES, changelogLead, changelogLeadRefusal, firstChangelogSection, newChangelogEntries, readChangelogEntries } from "./core.mjs";
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
  if (!section.released) {
    for (const entry of newChangelogEntries(before, after)) {
      const refusal = `is written in CHANGELOG.md's ${section.heading} section: write it in a file of its own in ${CHANGELOG_ENTRIES}/, which the Release pull request adds to the release's notes (${CHANGELOG_ENTRIES}/README.md)`;
      failures.push({ file: "CHANGELOG.md", title: "Changelog entry", text: `the entry ${quote(entry)} ${refusal}` });
    }
  }
}

// An entry the base holds already, in its file or in CHANGELOG.md (moved out of it), is not this pull request's.
for (const { path, text, entry } of readChangelogEntries(ROOT)) {
  if (atBase(path) === text) continue;
  if (text.trim() === "") {
    failures.push({ file: path, title: "Changelog entry", text: `${path} is empty: write what changed for the person upgrading` });
    continue;
  }
  if (before?.includes(entry)) continue;
  const refusal = releaseNoteRefusal(entry);
  if (refusal !== null) failures.push({ file: path, title: "Changelog entry", text: `${path} ${quote(entry)} ${refusal}` });
}

for (const failure of failures) console.log(`::error file=${failure.file},title=${failure.title}::${failure.text}`);
console.log(failures.length === 0 ? "Release notes: every entry this pull request adds is written for the person upgrading." : `Release notes: ${failures.length} problem(s).`);
process.exit(failures.length === 0 ? 0 : 1);
