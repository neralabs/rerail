#!/usr/bin/env node
// Core's release notes are written for the person upgrading (docs/release.md, "Release notes"). On a pull request, each
// entry it adds to CHANGELOG.md's first section (one its base does not hold) stays at a sentence or two and names no
// ticket, pull request or commit; on the Release pull request, whose first section is dated, the section leads with a
// paragraph for the people who upgrade, written under `## Unreleased` on main. ci.yml's lint job runs it.
//
//   node scripts/release/core-changelog-check.mjs BASE   (BASE: the pull request's base commit, fetched)
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { changelogEntries, changelogLead, firstChangelogSection, newChangelogEntries, NO_CHANGELOG_ENTRIES } from "./core.mjs";
import { releaseNoteRefusal } from "./release-notes.mjs";
import { ROOT } from "./packs.mjs";

const base = process.argv[2];
if (base === undefined) throw new Error("usage: core-changelog-check.mjs BASE");

const after = readFileSync(join(ROOT, "CHANGELOG.md"), "utf8");
let before = null;
try {
  before = execFileSync("git", ["show", `${base}:CHANGELOG.md`], { cwd: ROOT, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
} catch {
  console.log(`${base} has no CHANGELOG.md: every entry of the first section is checked`);
}

/** Each failure, `{ title, text }`, as a GitHub annotation and a line. */
const failures = [];
const section = firstChangelogSection(after);
if (section !== null) {
  const lead = changelogLead(section.body);
  const entries = changelogEntries(section.body);
  if (section.released && lead === "" && entries.length > 0 && section.body !== NO_CHANGELOG_ENTRIES) {
    failures.push({
      title: "Release notes need a lead",
      text: `CHANGELOG.md's ${section.heading} section opens with an entry: lead it with a paragraph saying what this release gives the people who upgrade, written under \`## Unreleased\` on main (docs/release.md, "Release notes")`,
    });
  }
  const leadRefusal = lead === "" ? null : releaseNoteRefusal(lead, null);
  if (leadRefusal !== null) failures.push({ title: "Release notes lead", text: `CHANGELOG.md's ${section.heading} lead ${leadRefusal}` });
  for (const entry of newChangelogEntries(before, after)) {
    const refusal = releaseNoteRefusal(entry);
    if (refusal !== null) failures.push({ title: "Changelog entry", text: `the entry "${entry.replace(/\s+/gu, " ").slice(0, 60)}…" ${refusal}` });
  }
}

for (const failure of failures) console.log(`::error file=CHANGELOG.md,title=${failure.title}::${failure.text}`);
console.log(failures.length === 0 ? "Release notes: every entry this pull request adds is written for the person upgrading." : `Release notes: ${failures.length} problem(s).`);
process.exit(failures.length === 0 ? 0 : 1);
