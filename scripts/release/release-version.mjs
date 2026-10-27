#!/usr/bin/env node
// Writes the Release pull request's changes into the checkout, and its description to <body.md>: the version of every
// package that merging it releases.
//   - Packs: with changesets pending, `npm run version:packs` (each pack's version and CHANGELOG.md, then the lockfile).
//   - Core: when core's version is released (its `v<version>` tag exists) and anything core ships has changed since
//     that tag (coreChanges in core-paths.mjs), the next version (nextCoreVersion) in package.json and package-lock.json's
//     root entries, and the root CHANGELOG.md's unreleased section dated today as that version. A version not tagged
//     yet is a release under way (release-pr.yml tags it before this runs), so core is left alone.
// Writes `releasable` (true when anything changed) to $GITHUB_OUTPUT when it is set. Needs the `v*` tags fetched.
//
//   node scripts/release/release-version.mjs <body.md>
import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { changelogSection, CORE_NAME, coreTag, coreTagRefusal, nextCoreVersion, releaseChangelog, setLockVersion, setPackageVersion } from "./core.mjs";
import { coreChanges } from "./core-paths.mjs";
import { listPacks, readChangesets, ROOT } from "./packs.mjs";

/** A changelog section longer than this is linked rather than quoted: a pull request description holds 65,536 characters. */
const QUOTED_NOTES = 12_000;
/** How many of the core paths that changed the description lists. */
const LISTED_CHANGES = 20;

// A large buffer: the paths changed since the last core tag can run past Node's 1 MiB default (21,265 of them once the
// pack test data left git), which failed the run with ENOBUFS.
const run = (command, args) => execFileSync(command, args, { cwd: ROOT, encoding: "utf8", maxBuffer: 1 << 28 });
const git = (args) => run("git", args).trim();
const readText = (path) => readFileSync(join(ROOT, path), "utf8");

const [bodyPath] = process.argv.slice(2);
if (bodyPath === undefined) throw new Error("usage: release-version.mjs <body.md>");

const packsBefore = listPacks();
if (readChangesets().length > 0) execFileSync("npm", ["run", "version:packs"], { cwd: ROOT, stdio: "inherit" });
const packs = listPacks()
  .map((pack) => ({ ...pack, from: packsBefore.find((before) => before.id === pack.id)?.version ?? null }))
  .filter((pack) => pack.from !== pack.version);

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
  coreRefusal = coreTagRefusal(version, tags);
  if (coreRefusal !== null) {
    console.log(`::warning title=Core not released::${coreRefusal}`);
    return null;
  }
  const changelog = releaseChangelog(readText("CHANGELOG.md"), version, new Date().toISOString().slice(0, 10));
  writeFileSync(join(ROOT, "package.json"), setPackageVersion(readText("package.json"), version));
  writeFileSync(join(ROOT, "package-lock.json"), `${JSON.stringify(setLockVersion(JSON.parse(readText("package-lock.json")), version), null, 2)}\n`);
  writeFileSync(join(ROOT, "CHANGELOG.md"), changelog.text);
  console.log(`core ${current} -> ${version}: ${changes.length} shipped path(s) changed since ${tag}`);
  return { from: current, version, notes: changelog.notes, changes };
}

const core = versionCore();

/** A package's release notes for the description: quoted, or linked when too long to quote. */
function notes(title, path, version) {
  const section = changelogSection(readText(path), version);
  const body = section === "" ? "_No changelog entries._" : section.length > QUOTED_NOTES ? `Too long to quote here: see \`${path}\` in this pull request.` : section;
  return ["<details>", `<summary>${title}</summary>`, "", body, "", "</details>"];
}

const rows = [
  ...(core === null ? [] : [{ name: CORE_NAME, from: core.from, version: core.version, how: `\`${coreTag(core.version)}\` tag, then release.yml` }]),
  ...packs.map((pack) => ({ name: pack.name, from: pack.from ?? "new", version: pack.version, how: "release-packs.yml" })),
];
const lines = [];
if (coreRefusal !== null) {
  lines.push("> [!WARNING]", `> Core is not released by this pull request: ${coreRefusal}. Name a later version in \`CHANGELOG.md\`'s unreleased heading (\`## <version> (unreleased)\`) on \`main\`.`, "");
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
      lines.push("", "> [!WARNING]", `> \`CHANGELOG.md\` has no unreleased section, so ${core.version}'s says it has no changelog entries. To add notes, add a \`## Unreleased\` section on \`main\`: this pull request picks it up.`);
    }
    lines.push("", ...notes(`${CORE_NAME} ${core.version}`, "CHANGELOG.md", core.version));
  }
  for (const pack of packs) lines.push("", ...notes(`${pack.name} ${pack.version}`, `packs/${pack.id}/CHANGELOG.md`, pack.version));
}
lines.push("", "This pull request is rewritten from `main` on every push to it (.github/workflows/release-pr.yml).");
writeFileSync(bodyPath, `${lines.join("\n")}\n`);

if (process.env.GITHUB_OUTPUT !== undefined) appendFileSync(process.env.GITHUB_OUTPUT, `releasable=${rows.length > 0}\n`);
if (rows.length === 0) console.log("Nothing to release.");
