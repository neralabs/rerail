#!/usr/bin/env node
// Writes the Release pull request's changes into the checkout, and its description to <body.md>: the version of every
// package that merging it releases.
//   - Packs: with changesets pending, `npm run version:packs` (each pack's version, then the lockfile), then each
//     released pack's CHANGELOG.md section for it, dated today: what changed in its rules since the commit that released
//     its previous version, and the changesets' entries by change type (pack-changelog.mjs). A pending changeset with
//     no type fails the run, naming it: one `changeset version` bumps every pack, so no pack can wait for its rename.
//   - Core: when core's version is released (its `v<version>` tag exists) and anything core ships has changed since
//     that tag (coreChanges in core-paths.mjs), the next version (nextCoreVersion) in package.json and package-lock.json's
//     root entries, the installer plugins' pins of it (PLUGIN_PINS), and the root CHANGELOG.md's section for it, dated
//     today: the lead under the unreleased heading and the entries pending in .changelog/ by change type, whose files it
//     deletes. A version not tagged yet is a release under way (release-pr.yml tags it before this runs), so core is
//     left alone.
// The description ends with the checklist whoever merges it ticks first (BEFORE_MERGING; docs/release.md, "Before
// merging"). It is written unticked each time, so a push to main clears it.
// Writes `releasable` (true when anything changed) to $GITHUB_OUTPUT when it is set. Needs the `v*` tags and the
// packs' release tags fetched, with the history back to each pack's last release.
//
//   node scripts/release/release-version.mjs <body.md>
import { execFileSync } from "node:child_process";
import { appendFileSync, existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, posix } from "node:path";
import { changelogEntryTypeRefusal, changelogLead, changelogSection, PLUGIN_PINS, CORE_NAME, coreTag, coreTagRefusal, nextCoreVersion, readChangelogEntries, releaseChangelog, setPluginVersion, setLockVersion, setPackageVersion } from "./core.mjs";
import { coreChanges } from "./core-paths.mjs";
import { packRuleChanges, packRuleNotes, packRules, releasedPackChangelog } from "./pack-changelog.mjs";
import { changesetNotes, changesetTypeRefusal, listPacks, readChangesets, ROOT } from "./packs.mjs";

/**
 * The most of the description the changelog sections quoted in it may take: a pull request description holds 65,536
 * characters, and the table, the core paths, the warnings and the checklist fit in the rest. Past it, the longest are
 * linked instead.
 */
const QUOTED_NOTES = 58_000;
/** How many of the core paths that changed the description lists. */
const LISTED_CHANGES = 20;
/**
 * What whoever merges the Release pull request confirms first, each a box to tick in its description. A pull request's
 * author confirms the same of their own change (.github/pull_request_template.md); this is the check of the release
 * as a whole.
 */
const BEFORE_MERGING = [
  "**End-user testing.** One hour using the staged release the way a user does. Core: `gh workflow run release.yml --ref changeset-release/main` builds it from this branch as the `rerail-package` run artifact, unpublished. Packs: this pull request's Release packs run uploads their tarballs as `rule-packs`, and `rerail packs add <tarball>` installs one (docs/release.md, \"Before merging\").",
  "**Release notes.** The notes above are accurate and complete, and each says what changed for the person upgrading.",
  "**Documentation.** `README.md` and `docs/` are accurate for what this releases: nothing merged since the last release left them behind.",
  "**`context7.json`.** Its folders, exclusions and rules match the docs and commands as they are now, so AI tools pull current Rerail docs.",
  "**`docs/internal-testing.md`.** It lists exactly the features in internal testing, and marks each one graduated or removed.",
  "**Scrub.** No internal name, customer name, internal URL or old product name (such as \"Tempo\") in the code, the docs, or the commit messages and pull request text since the last release. `npm run check` catches only internal ticket IDs and a few stale paths in tracked files.",
  "**npm package pages.** `README.md`, and `keywords` and `repository` in `package.json`, are current for core and every pack.",
];

// A large buffer: the paths changed since the last core tag can run past Node's 1 MiB default (21,265 of them once the
// pack test data left git), which failed the run with ENOBUFS.
const run = (command, args) => execFileSync(command, args, { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28 });
const git = (args) => run("git", args).trim();
const readText = (path) => readFileSync(join(ROOT, path), "utf8");
const today = new Date().toISOString().slice(0, 10);

/** The text of each git object `<commit>:<path>` in `objects`, null for one git cannot find, in one `git cat-file`. */
function readObjects(objects) {
  if (objects.length === 0) return [];
  const output = execFileSync("git", ["cat-file", "--batch"], { cwd: ROOT, input: `${objects.join("\n")}\n`, maxBuffer: 1 << 28 });
  const texts = [];
  let at = 0;
  for (let index = 0; index < objects.length; index += 1) {
    const end = output.indexOf(10, at);
    const size = /^\S+ blob (\d+)$/u.exec(output.subarray(at, end).toString("utf8"))?.[1];
    at = end + 1;
    if (size === undefined) {
      texts.push(null);
      continue;
    }
    texts.push(output.subarray(at, at + Number(size)).toString("utf8"));
    at += Number(size) + 1;
  }
  return texts;
}

/**
 * The commit that released `pack`'s previous version (`from`): the last to set its package.json's version to it,
 * whether or not its release tag exists yet (its publish may wait for approval). Null for a pack never released, which
 * has no release tag at all, and for one new to the checkout.
 */
function releasedCommit(pack) {
  if (pack.from === null || git(["tag", "--list", `${pack.name}@*`]) === "") return null;
  const version = `"version": "${pack.from.replaceAll(".", "\\.")}"`;
  const commit = git(["log", "-1", "--format=%H", "-G", version, "--", `packs/${pack.id}/package.json`]);
  if (commit === "") throw new Error(`no commit sets ${pack.name}'s version to ${pack.from}: fetch the full history`);
  return commit;
}

/** `pack`'s rules (packRules) at `commit`, or in the checkout for null; none where it has no pack.json. */
function rulesAt(pack, commit) {
  const directory = `packs/${pack.id}`;
  const read = (paths) => (commit === null ? paths.map(readText) : readObjects(paths.map((path) => `${commit}:${path}`)));
  const [manifest] = read([`${directory}/pack.json`]);
  if (manifest === null) return [];
  const parsed = JSON.parse(manifest);
  const entries = parsed.rules ?? [];
  const documents = read(entries.map((rule) => posix.join(directory, rule.document)));
  const byId = new Map(entries.map((rule, index) => [rule.id, documents[index] === null ? undefined : JSON.parse(documents[index])]));
  const rules = packRules(parsed, (rule) => byId.get(rule.id));
  const unmoded = rules.find((rule) => rule.start === undefined);
  if (unmoded !== undefined) throw new Error(`${directory}: its document gives ${unmoded.id} no mode${commit === null ? "" : ` at ${commit}`}`);
  return rules;
}

const [bodyPath] = process.argv.slice(2);
if (bodyPath === undefined) throw new Error("usage: release-version.mjs <body.md>");

const packsBefore = listPacks();
// `changeset version` deletes the changesets, so they are read first.
const changesets = readChangesets();
const untypedChangeset = changesets.find(({ file }) => changesetTypeRefusal(file) !== null);
if (untypedChangeset !== undefined) throw new Error(`on main, .changeset/${untypedChangeset.file} ${changesetTypeRefusal(untypedChangeset.file)}`);
if (changesets.length > 0) execFileSync("npm", ["run", "version:packs"], { cwd: ROOT, stdio: "inherit" });
const packs = listPacks()
  .map((pack) => ({ ...pack, from: packsBefore.find((before) => before.id === pack.id)?.version ?? null }))
  .filter((pack) => pack.from !== pack.version);
for (const pack of packs) {
  const path = `packs/${pack.id}/CHANGELOG.md`;
  // A pack never released has no earlier rules, so every rule it ships is new; rulesAt(pack, null) would read the checkout.
  const commit = releasedCommit(pack);
  const changes = packRuleChanges(commit === null ? [] : rulesAt(pack, commit), rulesAt(pack, null));
  const changelog = existsSync(join(ROOT, path)) ? readText(path) : "";
  writeFileSync(join(ROOT, path), releasedPackChangelog(changelog, pack.name, pack.version, today, packRuleNotes(changes), changesetNotes(changesets, pack.name)));
}

/** Why core's next version can't be released (release-pr.yml's tag-core would refuse its tag), or null. */
let coreRefusal = null;

/** Core's bump, `{ from, version, notes, changes }`, or null when core is not released by this pull request. */
function versionCore() {
  const current = JSON.parse(readText("package.json")).version;
  const tag = coreTag(current);
  const tags = git(["tag", "--list", "v*"]).split("\n");
  if (!tags.includes(tag)) {
    console.log(`core ${current} has no ${tag} tag yet: its release is under way, so core is not bumped`);
    return null;
  }
  const changed = git(["diff", "--name-only", "--no-renames", tag, "HEAD"]).split("\n").filter((path) => path !== "");
  const changes = coreChanges(changed, (ref) => git(["show", `${ref === "release" ? tag : "HEAD"}:package-lock.json`]));
  if (changes.length === 0) {
    console.log(`core: nothing it ships has changed since ${tag}`);
    return null;
  }
  const version = nextCoreVersion(current, readText("CHANGELOG.md"));
  // A version tag-core would refuse is left out, so the packs still release; the description says why.
  const tagRefusal = coreTagRefusal(version, tags);
  const pending = readChangelogEntries(ROOT);
  // So is one with a pending entry that has no type, from a pull request that merged without one: its notes have no heading for it.
  const untyped = pending.filter(({ path }) => changelogEntryTypeRefusal(path) !== null);
  if (tagRefusal !== null) coreRefusal = `${tagRefusal}. Name a later version in \`CHANGELOG.md\`'s unreleased heading (\`## <version> (unreleased)\`) on \`main\``;
  else if (untyped.length > 0) coreRefusal = `on \`main\`, ${untyped.map(({ path }) => `\`${path}\``).join(", ")} ${changelogEntryTypeRefusal(untyped[0].path)}`;
  if (coreRefusal !== null) {
    console.log(`::warning title=Core not released::${coreRefusal}`);
    return null;
  }
  const changelog = releaseChangelog(readText("CHANGELOG.md"), version, today, pending);
  writeFileSync(join(ROOT, "package.json"), setPackageVersion(readText("package.json"), version));
  writeFileSync(join(ROOT, "package-lock.json"), `${JSON.stringify(setLockVersion(JSON.parse(readText("package-lock.json")), version), null, 2)}\n`);
  for (const path of PLUGIN_PINS) writeFileSync(join(ROOT, path), setPluginVersion(readText(path), version));
  writeFileSync(join(ROOT, "CHANGELOG.md"), changelog.text);
  for (const { path } of pending) rmSync(join(ROOT, path));
  console.log(`core ${current} -> ${version}: ${changes.length} shipped path(s) changed since ${tag}`);
  return { from: current, version, notes: changelog.notes, changes };
}

const core = versionCore();

/** Each released package's notes for the description: `{ title, path, section }`, core's first. */
const released = [
  ...(core === null ? [] : [{ title: `${CORE_NAME} ${core.version}`, path: "CHANGELOG.md", version: core.version }]),
  ...packs.map((pack) => ({ title: `${pack.name} ${pack.version}`, path: `packs/${pack.id}/CHANGELOG.md`, version: pack.version })),
].map(({ title, path, version }) => ({ title, path, section: changelogSection(readText(path), version) }));
/** The sections linked rather than quoted: the longest, until the rest fit in QUOTED_NOTES. */
const linked = new Set();
let quotedLength = released.reduce((sum, { section }) => sum + section.length, 0);
for (const note of [...released].sort((a, b) => b.section.length - a.section.length)) {
  if (quotedLength <= QUOTED_NOTES) break;
  linked.add(note);
  quotedLength -= note.section.length;
}

/** A package's release notes for the description: quoted, or linked when they did not fit. */
function notes(note) {
  const body = note.section === "" ? "_No changelog entries._" : linked.has(note) ? `Too long to quote here: see \`${note.path}\` in this pull request.` : note.section;
  return ["<details>", `<summary>${note.title}</summary>`, "", body, "", "</details>"];
}

const rows = [
  ...(core === null ? [] : [{ name: CORE_NAME, from: core.from, version: core.version, how: `\`${coreTag(core.version)}\` tag, then release.yml` }]),
  ...packs.map((pack) => ({ name: pack.name, from: pack.from ?? "new", version: pack.version, how: "release-packs.yml" })),
];
const lines = [];
if (coreRefusal !== null) {
  lines.push("> [!WARNING]", `> Core is not released by this pull request: ${coreRefusal}.`, "");
}
if (rows.length === 0) {
  lines.push("Nothing to release.");
} else {
  lines.push(
    "Merging this pull request releases these packages to npm under `latest` (each publish waits for a reviewer to approve the `release` environment):",
    "",
    "| Package | Version | Published by |",
    "|---|---|---|",
    ...rows.map((row) => `| \`${row.name}\` | ${row.from} → ${row.version} | ${row.how} |`),
  );
  if (core !== null) {
    const listed = core.changes.slice(0, LISTED_CHANGES).map((path) => `\`${path}\``).join(", ");
    const more = core.changes.length > LISTED_CHANGES ? `, and ${core.changes.length - LISTED_CHANGES} more` : "";
    lines.push("", `Core is released because what it ships changed since \`${coreTag(core.from)}\`: ${listed}${more}.`);
    if (!core.notes) {
      lines.push("", "> [!WARNING]", `> No changelog entry is pending in \`.changelog/\` and \`CHANGELOG.md\` has no unreleased notes, so ${core.version}'s says it has no changelog entries. To add notes, add entries in \`.changelog/\` on \`main\`: this pull request picks them up.`);
    } else if (changelogLead(changelogSection(readText("CHANGELOG.md"), core.version)) === "") {
      lines.push("", "> [!WARNING]", `> ${core.version}'s notes open with an entry and have no lead: CI fails this pull request until a paragraph saying what the release gives the people who upgrade is written at the top of \`## Unreleased\` on \`main\` (docs/release.md, "Release notes"). This pull request picks it up.`);
    }
  }
  for (const note of released) lines.push("", ...notes(note));
  lines.push(
    "",
    "## Before merging",
    "",
    "Whoever merges this ticks each box first. A push to `main` rewrites this description and clears them, because what you checked is no longer what merging releases: check again.",
    "",
    ...BEFORE_MERGING.map((item) => `- [ ] ${item}`),
  );
}
lines.push("", "This pull request is rewritten from `main` on every push to it (.github/workflows/release-pr.yml).");
writeFileSync(bodyPath, `${lines.join("\n")}\n`);

if (process.env.GITHUB_OUTPUT !== undefined) appendFileSync(process.env.GITHUB_OUTPUT, `releasable=${rows.length > 0}\n`);
if (rows.length === 0) console.log("Nothing to release.");
