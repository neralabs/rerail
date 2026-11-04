// scripts/release/core.mjs, core-paths.mjs and dist-tags.mjs: when core is released (what it ships changed since its
// last tag), the version it takes, the changelog and version edits the Release pull request makes, the tag guard,
// `next` kept from trailing `latest`, the refusal of a preview older than `latest`, and the check of a release's
// dist-tags.

import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import semver from "semver";
import { describe, expect, it } from "vitest";
import { coreChanges, lockChangesCore, matrixChanges, shipsWithCore } from "../../scripts/release/core-paths.mjs";
import { NO_CHANGELOG_ENTRIES, PLUGIN_PINS, changelogEntries, changelogEntryFileRefusal, changelogLead, changelogLeadRefusal, changelogSection, coreTagRefusal, firstChangelogSection, handWrittenChangelogEntries, newChangelogEntries, nextCoreVersion, readChangelogEntries, releaseChangelog, setPluginVersion, setLockVersion, setPackageVersion, unreleasedChangelogEntries } from "../../scripts/release/core.mjs";
import { RELEASE_NOTE_WORDS, releaseNoteRefusal } from "../../scripts/release/release-notes.mjs";
import { compareVersions, distTagReport, nextTrails, parseDistTagList, parseExpectation, previewRefusal, raiseNext } from "../../scripts/release/dist-tags.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

describe("shipsWithCore", () => {
  it("counts the source, engine, build inputs and package files, and not the packs, docs, CI, tests or dev scripts", () => {
    for (const path of ["src/cli/cli.ts", "native/src/lib.rs", "languages/python.json", "ui/src/App.tsx", "package.json", "Cargo.lock", "scripts/build/copy-ui.mjs", "skills/rerail/SKILL.md", "README.md"]) {
      expect(shipsWithCore(path), path).toBe(true);
    }
    for (const path of ["packs/python/rules/x.json", "docs/pack-format.md", "scripts/build/docs-site.mjs", ".github/workflows/ci.yml", ".changeset/a.md", ".changelog/a.md", "test/scripts/x.test.mjs", "integration/x.test.ts", "scripts/release/core.mjs", "scripts/dev/rerail-dev.mjs", "CHANGELOG.md", "CONTRIBUTING.md"]) {
      expect(shipsWithCore(path), path).toBe(false);
    }
  });
});

describe("lockChangesCore and coreChanges", () => {
  const lock = (packs, root = "1.0.0") => JSON.stringify({ version: "0.1.0", packages: { "": { version: "0.1.0" }, "node_modules/semver": { version: root }, "packs/python": { version: packs } } });

  it("leaves out a lockfile change only to pack workspace entries, as the pack release makes", () => {
    expect(lockChangesCore(lock("0.1.3"), lock("0.1.4"))).toBe(false);
    expect(lockChangesCore(lock("0.1.3"), lock("0.1.3", "1.0.1"))).toBe(true);
  });

  it("keeps the shipped paths, reading the lockfile only when it changed", () => {
    const reads = [];
    const readLock = (ref) => (reads.push(ref), ref === "release" ? lock("0.1.3") : lock("0.1.4"));
    expect(coreChanges(["packs/python/package.json", "docs/x.md"], readLock)).toEqual([]);
    expect(reads).toEqual([]);
    expect(coreChanges(["package-lock.json", "src/a.ts", "packs/python/package.json"], readLock)).toEqual(["src/a.ts"]);
    expect(reads).toEqual(["release", "head"]);
  });
});

describe("matrixChanges", () => {
  const lock = (packs, root = "1.0.0") => JSON.stringify({ version: "0.1.0", packages: { "": { version: "0.1.0" }, "node_modules/semver": { version: root }, "packs/python": { version: packs } } });
  const readLock = (ref) => (ref === "release" ? lock("0.1.3") : lock("0.1.4"));

  it("skips a push that changes only packs, docs, tests or other CI, or the lockfile's pack entries", () => {
    expect(matrixChanges(["packs/react/pack.json", "docs/environment-matrix.md", "test/a.test.ts", ".github/workflows/ci.yml", "package-lock.json"], readLock))
      .toEqual({ core: [], matrix: [] });
  });

  it("runs for what core ships and for the matrix's own files, a directory by its prefix and a file only by its exact path", () => {
    expect(matrixChanges(["src/cli/cli.ts", "scripts/host/azure/guest.sh", "scripts/host/windows-host.ps1", ".github/workflows/env-matrix.yml"], readLock))
      .toEqual({ core: ["src/cli/cli.ts"], matrix: ["scripts/host/azure/guest.sh", "scripts/host/windows-host.ps1", ".github/workflows/env-matrix.yml"] });
    expect(matrixChanges(["scripts/host/windows-host.ps1.bak", "scripts/host/azure-notes.md", ".github/workflows/env-matrix.yml.orig"], readLock).matrix).toEqual([]);
  });
});

describe("nextCoreVersion", () => {
  it("takes a prerelease to its release and a release to its next patch", () => {
    expect(nextCoreVersion("0.1.0-next.4", "# Changelog\n\n## 0.1.0 (unreleased)\n\n- a\n")).toBe("0.1.0");
    expect(nextCoreVersion("0.1.0", "# Changelog\n\n## Unreleased\n\n- a\n\n## 0.1.0 (2026-10-22)\n")).toBe("0.1.1");
    expect(nextCoreVersion("0.1.1", "# Changelog\n\n## 0.1.1 (2026-10-23)\n")).toBe("0.1.2");
  });

  it("takes the later version an unreleased heading names, and refuses one that is not later or not a version", () => {
    expect(nextCoreVersion("0.1.4", "## 0.2.0 (unreleased)\n")).toBe("0.2.0");
    expect(() => nextCoreVersion("0.1.4", "## 0.1.4 (unreleased)\n")).toThrow("not later than core's 0.1.4");
    expect(() => nextCoreVersion("0.1.4", "## soon (unreleased)\n")).toThrow("soon, which is not a version");
    expect(() => nextCoreVersion("0.1.4", "## v0.2.0 (unreleased)\n")).toThrow("v0.2.0, which is not a version");
  });

  it("reads a Keep a Changelog `[Unreleased]` heading as unreleased", () => {
    expect(nextCoreVersion("0.1.0", "# Changelog\n\n## [Unreleased]\n\n- a\n")).toBe("0.1.1");
    expect(releaseChangelog("## [Unreleased]\n\n- a\n", "0.1.1", "2026-10-23", [])).toEqual({ text: "## 0.1.1 (2026-10-23)\n\n- a\n", notes: true });
  });
});

describe("releaseChangelog and changelogSection", () => {
  it("dates the unreleased section as the release, whatever its heading", () => {
    const named = releaseChangelog("# Changelog\n\n## 0.1.0 (unreleased)\n\n- a\n", "0.1.0", "2026-10-22", []);
    expect(named).toEqual({ text: "# Changelog\n\n## 0.1.0 (2026-10-22)\n\n- a\n", notes: true });
    const plain = releaseChangelog("# Changelog\n\n## Unreleased\n\n- b\n\n## 0.1.0 (2026-10-22)\n\n- a\n", "0.1.1", "2026-10-23", []);
    expect(plain.text).toBe("# Changelog\n\n## 0.1.1 (2026-10-23)\n\n- b\n\n## 0.1.0 (2026-10-22)\n\n- a\n");
  });

  it("puts the pending entries under the unreleased lead, in their order, above the last release", () => {
    const text = "# Changelog\n\n## Unreleased\n\nA lead.\n\n## 0.1.0 (2026-10-22)\n\n- a\n";
    const released = releaseChangelog(text, "0.1.1", "2026-10-23", ["- b", "- c\n\n  More on c."]);
    expect(released).toEqual({ text: "# Changelog\n\n## 0.1.1 (2026-10-23)\n\nA lead.\n\n- b\n- c\n\n  More on c.\n\n## 0.1.0 (2026-10-22)\n\n- a\n", notes: true });
    expect(changelogEntries(changelogSection(released.text, "0.1.1"))).toEqual(["- b", "- c\n\n  More on c."]);
  });

  it("dates pending entries with no unreleased heading as a section with no lead, which CI then asks for", () => {
    const released = releaseChangelog("# Changelog\n\n## 0.1.0 (2026-10-22)\n\n- a\n", "0.1.1", "2026-10-23", ["- b"]);
    expect(released).toEqual({ text: "# Changelog\n\n## 0.1.1 (2026-10-23)\n\n- b\n\n## 0.1.0 (2026-10-22)\n\n- a\n", notes: true });
    expect(changelogLeadRefusal(firstChangelogSection(released.text))).toContain("opens with an entry");
  });

  it("adds a section saying there are no notes when nobody wrote a lead or an entry", () => {
    const released = releaseChangelog("# Changelog\n\n## 0.1.0 (2026-10-22)\n\n- a\n", "0.1.1", "2026-10-23", []);
    expect(released).toEqual({ text: "# Changelog\n\n## 0.1.1 (2026-10-23)\n\nNo changelog entries were written for this release.\n\n## 0.1.0 (2026-10-22)\n\n- a\n", notes: false });
    expect(changelogSection(released.text, "0.1.1")).toBe("No changelog entries were written for this release.");
    expect(releaseChangelog("# Changelog\n\n## Unreleased\n\n## 0.1.0 (2026-10-22)\n\n- a\n", "0.1.1", "2026-10-23", [])).toEqual(released);
  });

  it("reads the pending entries in file-name order, each as the bullet it becomes, and nothing else in the directory", () => {
    const root = mkdtempSync(join(tmpdir(), "rerail-changelog-"));
    try {
      expect(readChangelogEntries(root)).toEqual([]);
      mkdirSync(join(root, ".changelog"));
      writeFileSync(join(root, ".changelog", "dashboard-b.md"), "The dashboard says less.\n\nIt says it by rule.\n");
      writeFileSync(join(root, ".changelog", "cli-a.md"), "`rerail version` prints the version.\n");
      writeFileSync(join(root, ".changelog", "README.md"), "# Changelog entries\n");
      writeFileSync(join(root, ".changelog", "notes.txt"), "not an entry\n");
      expect(readChangelogEntries(root).map(({ path, entry }) => [path, entry])).toEqual([
        [".changelog/cli-a.md", "- `rerail version` prints the version."],
        [".changelog/dashboard-b.md", "- The dashboard says less.\n\n  It says it by rule."],
      ]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("reads one version's section, dated as core's or bare as a pack's, and nothing for a version it lacks", () => {
    const text = "# Changelog\n\n## 0.1.4\n\n### Patch Changes\n\n- b\n\n## 0.1.3 (2026-10-01)\n\n- a\n";
    expect(changelogSection(text, "0.1.4")).toBe("### Patch Changes\n\n- b");
    expect(changelogSection(text, "0.1.3")).toBe("- a");
    expect(changelogSection(text, "0.1.2")).toBe("");
  });
});

describe("release notes: the lead, the entries, and what a note may say", () => {
  const released = ["# Changelog", "", "## 0.1.1 (2026-11-02)", "", "Rerail now reads Vue components.", "The daemon needs no login.", "", "### Rules", "", "- Vue is a code-rule language.", "  Its scripts are ECMAScript.", "- A `using` declaration binds.", "", "## 0.1.0 (2026-10-27)", "", "First release.", ""].join("\n");
  const unreleased = "# Changelog\n\n## Unreleased\n\n- A new entry.\n- An old entry.\n\n## 0.1.0 (2026-10-27)\n\n- An old entry.\n";

  it("reads the first section, whether released or not", () => {
    expect(firstChangelogSection(released)).toEqual({ heading: "0.1.1 (2026-11-02)", released: true, body: "Rerail now reads Vue components.\nThe daemon needs no login.\n\n### Rules\n\n- Vue is a code-rule language.\n  Its scripts are ECMAScript.\n- A `using` declaration binds." });
    expect(firstChangelogSection(unreleased)).toMatchObject({ heading: "Unreleased", released: false });
    expect(firstChangelogSection("# Changelog\n\n## 0.2.0 (unreleased)\n\n- a\n")).toMatchObject({ released: false });
    expect(firstChangelogSection("# Changelog\n")).toBeNull();
  });

  it("reads the lead as the prose before the first entry or heading, and none when the section opens with one", () => {
    expect(changelogLead(firstChangelogSection(released).body)).toBe("Rerail now reads Vue components.\nThe daemon needs no login.");
    expect(changelogLead("- An entry.")).toBe("");
    expect(changelogLead("### Rules\n\n- An entry.")).toBe("");
    expect(changelogLead("Only a lead.")).toBe("Only a lead.");
  });

  it("reads each entry with the lines indented under it, under any heading", () => {
    expect(changelogEntries(firstChangelogSection(released).body)).toEqual(["- Vue is a code-rule language.\n  Its scripts are ECMAScript.", "- A `using` declaration binds."]);
    expect(changelogEntries("No entries.")).toEqual([]);
  });

  it("keeps the blank lines between an entry's paragraphs, as changesets writes them, so the entry is found again as written", () => {
    const section = "A lead.\n\n- A first paragraph.\n\n  A second paragraph.\n\n\n  A third.\n\n- Another entry.\n\nTrailing prose.\n";
    const entries = changelogEntries(section);
    expect(entries).toEqual(["- A first paragraph.\n\n  A second paragraph.\n\n\n  A third.", "- Another entry."]);
    const before = `# Changelog\n\n## Unreleased\n\n${section}`;
    expect(newChangelogEntries(before, before)).toEqual([]);
  });

  it("asks a released section that opens with an entry for a lead, and nothing else", () => {
    expect(changelogLeadRefusal(firstChangelogSection("# C\n\n## 0.1.1 (2026-11-02)\n\n- An entry.\n"))).toContain("0.1.1 (2026-11-02) section opens with an entry");
    expect(changelogLeadRefusal(firstChangelogSection("# C\n\n## 0.1.1 (2026-11-02)\n\n### Rules\n\n- An entry.\n"))).toContain("opens with an entry");
    expect(changelogLeadRefusal(firstChangelogSection("# C\n\n## Unreleased\n\n- An entry.\n"))).toBeNull();
    expect(changelogLeadRefusal(firstChangelogSection("# C\n\n## 0.2.0 (unreleased)\n\n- An entry.\n"))).toBeNull();
    expect(changelogLeadRefusal(firstChangelogSection(released))).toBeNull();
    expect(changelogLeadRefusal(firstChangelogSection(`# C\n\n## 0.1.1 (2026-11-02)\n\n${NO_CHANGELOG_ENTRIES}\n`))).toBeNull();
  });

  it("tells the entries a pull request adds from the ones its base already had", () => {
    expect(newChangelogEntries("# Changelog\n\n## Unreleased\n\n- An old entry.\n", unreleased)).toEqual(["- A new entry."]);
    expect(newChangelogEntries(null, unreleased)).toEqual(["- A new entry.", "- An old entry."]);
    expect(newChangelogEntries("", "# Changelog\n")).toEqual([]);
  });

  it("refuses an entry written into CHANGELOG.md by hand, under a dated heading too, but not the Release pull request's", () => {
    const dated = "# Changelog\n\n## 0.1.1 (2026-11-02)\n\nA lead.\n\n- A released entry.\n";
    const addedToDated = dated.replace("- A released entry.", "- A released entry.\n- A hand-written entry.");
    expect(handWrittenChangelogEntries(dated, addedToDated)).toEqual(["- A hand-written entry."]);
    expect(handWrittenChangelogEntries("# Changelog\n\n## Unreleased\n\n- An old entry.\n", unreleased)).toEqual(["- A new entry."]);
    const releasePr = releaseChangelog("# Changelog\n\n## Unreleased\n\nA lead.\n", "0.1.1", "2026-11-02", ["- A moved entry."]).text;
    expect(handWrittenChangelogEntries("# Changelog\n\n## Unreleased\n\nA lead.\n", releasePr)).toEqual([]);
    const nextRelease = releaseChangelog(dated, "0.1.2", "2026-11-09", ["- A moved entry."]).text;
    expect(handWrittenChangelogEntries(dated, nextRelease)).toEqual([]);
  });

  it("reads the whole entries of an unreleased first section only, as moved out of it", () => {
    expect(unreleasedChangelogEntries("# Changelog\n\n## Unreleased\n\n- A pending entry.\n\n## 0.1.0 (2026-10-27)\n\n- A released entry.\n")).toEqual(["- A pending entry."]);
    expect(unreleasedChangelogEntries(released)).toEqual([]);
    expect(unreleasedChangelogEntries(null)).toEqual([]);
  });

  it("refuses an entry file that is empty or opens with a list marker, which the Release pull request would nest", () => {
    expect(changelogEntryFileRefusal("The dashboard says less.\n")).toBeNull();
    expect(changelogEntryFileRefusal("Rules read `-1` and `*` as written.\n")).toBeNull();
    expect(changelogEntryFileRefusal(" \n")).toBe("is empty: write what changed for the person upgrading");
    expect(changelogEntryFileRefusal("- The dashboard says less.\n")).toContain("starts with `- `");
    expect(changelogEntryFileRefusal("\n* The dashboard says less.\n")).toContain("starts with `* `");
  });

  // The ticket id is spelled in pieces: check-internal-refs.mjs refuses it written out in any tracked file.
  const ticket = ["NER", "2217"].join("-");

  it("refuses a note that runs long or names a ticket, a pull request or a commit, and takes one written for the person upgrading", () => {
    expect(releaseNoteRefusal("- Rerail reads Vue components: a rule names tree-sitter-vue's kinds.")).toBeNull();
    expect(releaseNoteRefusal(`- ${"word ".repeat(RELEASE_NOTE_WORDS)}`)).toBeNull();
    expect(releaseNoteRefusal(`- ${"word ".repeat(RELEASE_NOTE_WORDS + 1)}`)).toBe(`runs to ${RELEASE_NOTE_WORDS + 1} words; a release note says what changed in a sentence or two (at most ${RELEASE_NOTE_WORDS} words)`);
    expect(releaseNoteRefusal(`- ${ticket}: licences are checked at the commit.`)).toBe(`names the ticket ${ticket}, which says nothing to the person upgrading`);
    expect(releaseNoteRefusal(`- Licences (${ticket.toLowerCase()}) are checked.`)).toContain("names the ticket");
    expect(releaseNoteRefusal("- The renamer (#360) keeps literals.")).toBe("names pull request #360, which says nothing to the person upgrading");
    expect(releaseNoteRefusal("- e06db22: Messages.")).toBe("names the commit e06db22, which says nothing to the person upgrading");
    expect(releaseNoteRefusal("- Rerail 0.1.0 reads `#!/usr/bin/env node` and `#[derive]`, and sha256 digests of 1.2.3.")).toBeNull();
  });

  it("takes a number that is no hash, and a # that is no pull request", () => {
    expect(releaseNoteRefusal("- Files over 10485760 bytes are skipped.")).toBeNull();
    expect(releaseNoteRefusal("- CVE-2024-1234567 is fixed.")).toBeNull();
    expect(releaseNoteRefusal("- Fixes the #1 complaint: slow start.")).toBeNull();
    expect(releaseNoteRefusal("- The brand colour is #4b3fa6; the digest `#1a2b3c` is not a pull request.")).toBeNull();
    expect(releaseNoteRefusal("- See #360, then #361.")).toBe("names pull request #360, which says nothing to the person upgrading");
    expect(releaseNoteRefusal("- Landed in abcdef0.")).toBe("names the commit abcdef0, which says nothing to the person upgrading");
  });

  it("takes a standard's or licence's name, and a CVE id, for what they are, not a ticket", () => {
    expect(releaseNoteRefusal("- Digests use SHA-256; files are read as UTF-8; LGPL-2.1 and BSD-3-Clause packs are refused; CVE-2026-1234 is fixed.")).toBeNull();
  });

  it("caps no words on a lead, only what it names", () => {
    expect(releaseNoteRefusal("word ".repeat(RELEASE_NOTE_WORDS + 10), null)).toBeNull();
    expect(releaseNoteRefusal(`See ${ticket}.`, null)).toContain(ticket);
  });
});

describe("setPackageVersion and setLockVersion", () => {
  it("changes only the root version, as a hand bump's three-line diff did", () => {
    const text = '{\n  "name": "@neralabs/rerail",\n  "version": "0.1.0-next.4",\n  "dependencies": {\n    "x": {\n      "version": "1.0.0"\n    }\n  }\n}\n';
    expect(setPackageVersion(text, "0.1.0")).toBe(text.replace('"0.1.0-next.4"', '"0.1.0"'));
    const lock = { name: "@neralabs/rerail", version: "0.1.0-next.4", packages: { "": { name: "@neralabs/rerail", version: "0.1.0-next.4" }, "packs/python": { version: "0.1.4" } } };
    expect(setLockVersion(lock, "0.1.0")).toEqual({ ...lock, version: "0.1.0", packages: { ...lock.packages, "": { name: "@neralabs/rerail", version: "0.1.0" } } });
  });
});

describe("the installer plugins' pins of core", () => {
  const read = (path) => readFileSync(join(ROOT, path), "utf8");

  it("pin the version core's package.json names, so the directory's installer runs the release it ships with", () => {
    const { version } = JSON.parse(read("package.json"));
    for (const path of PLUGIN_PINS) {
      const pins = [...read(path).matchAll(/@neralabs\/rerail@(\S+?)[\s`]/gu)].map((match) => match[1]);
      if (path.endsWith("plugin.json")) expect(JSON.parse(read(path)).version, path).toBe(version);
      else expect(pins.length, path).toBeGreaterThan(0);
      for (const pin of pins) expect(pin, path).toBe(version);
    }
  });

  it("moves the manifest version and every npx pin, and leaves rule pack names alone", () => {
    const manifest = '{\n  "name": "rerail",\n  "version": "0.1.0",\n  "author": {\n    "version": "x"\n  }\n}\n';
    expect(setPluginVersion(manifest, "0.1.1")).toBe(manifest.replace('"0.1.0"', '"0.1.1"'));
    const skill = "Run `npx @neralabs/rerail@0.1.0 setup`, then:\n\nnpx --yes @neralabs/rerail@0.1.0 setup --relay\n\nnot @neralabs/rerail-pack-sql@0.1.4";
    expect(setPluginVersion(skill, "0.2.0")).toBe(skill.replaceAll("rerail@0.1.0", "rerail@0.2.0"));
  });
});

describe("coreTagRefusal", () => {
  const tags = ["v0.1.0-next.3", "v0.1.0-next.4"];

  it("tags a version later than every release, the first plain release after the prereleases included", () => {
    expect(coreTagRefusal("0.1.0", tags)).toBeNull();
    expect(coreTagRefusal("0.1.1", [...tags, "v0.1.0"])).toBeNull();
  });

  it("refuses a version already tagged, one moved backwards, and one that is not a version", () => {
    expect(coreTagRefusal("v0.2.0", tags)).toBe("package.json's version v0.2.0 is not a version");
    expect(coreTagRefusal("0.1.0-next.4", tags)).toBe("v0.1.0-next.4 already exists");
    expect(coreTagRefusal("0.1.0-next.2", tags)).toBe("0.1.0-next.2 is not later than the released 0.1.0-next.4");
    expect(coreTagRefusal("latest", tags)).toBe("package.json's version latest is not a version");
  });
});

describe("distTagReport", () => {
  const expected = ["@neralabs/rerail@0.1.0", "@neralabs/rerail-pack-python@0.1.5:next", "@neralabs/rerail-pack-react@0.1.5"].map(parseExpectation);

  it("passes a package whose dist-tag is the version just published, and fails one that is not or could not be read", () => {
    const distTags = { "@neralabs/rerail": { latest: "0.1.0", next: "0.1.0" }, "@neralabs/rerail-pack-python": { latest: "0.1.3", held: "0.1.3", next: "0.1.5" }, "@neralabs/rerail-pack-react": null };
    const report = distTagReport(expected, (name) => distTags[name]);
    expect(report.failures).toEqual(["@neralabs/rerail-pack-react: its dist-tags could not be read from npm"]);
    expect(report.table).toContain("| `@neralabs/rerail` | 0.1.0 | `latest` ✅ | `latest`: 0.1.0, `next`: 0.1.0 |");
    const stale = distTagReport([expected[0]], () => ({ latest: "0.1.0-next.4", next: "0.1.0-next.4" }));
    expect(stale.failures).toEqual(["@neralabs/rerail: latest is 0.1.0-next.4, not the 0.1.0 just published"]);
  });

  it("fails a latest or next release whose next is unset or older than latest, and passes a newer next or a beta release", () => {
    const trailing = distTagReport([expected[0]], () => ({ latest: "0.1.0", next: "0.1.0-next.4" }));
    expect(trailing.failures).toEqual(["@neralabs/rerail: next is 0.1.0-next.4, older than latest 0.1.0; a release under latest must move next up to it"]);
    expect(distTagReport([expected[0]], () => ({ latest: "0.1.0" })).failures).toEqual(["@neralabs/rerail: next is not set; a release under latest must move next up to it"]);
    expect(distTagReport([expected[0]], () => ({ latest: "0.1.0", next: "0.1.1-next.1" })).failures).toEqual([]);
    const preview = distTagReport([parseExpectation("@neralabs/rerail@0.0.9-next.1:next")], () => ({ latest: "0.1.0", next: "0.0.9-next.1" }));
    expect(preview.failures).toEqual(["@neralabs/rerail: next is 0.0.9-next.1, older than latest 0.1.0; a next preview must not be older than latest"]);
    const beta = distTagReport([parseExpectation("@neralabs/rerail@0.2.0-beta.1:beta")], () => ({ latest: "0.1.0", next: "0.1.0-next.4", beta: "0.2.0-beta.1" }));
    expect(beta.failures).toEqual([]);
  });

  it("reads npm dist-tag ls output", () => {
    expect(parseDistTagList("held: 0.1.3\nlatest: 0.1.3\nnext: 0.1.5\n")).toEqual({ held: "0.1.3", latest: "0.1.3", next: "0.1.5" });
  });

  it("reads a scoped name, its version and its dist-tag, latest by default", () => {
    expect(expected[0]).toEqual({ name: "@neralabs/rerail", version: "0.1.0", tag: "latest" });
    expect(expected[1]).toEqual({ name: "@neralabs/rerail-pack-python", version: "0.1.5", tag: "next" });
    expect(() => parseExpectation("@neralabs/rerail")).toThrow("is not <name>@<version>");
  });
});

describe("compareVersions and nextTrails", () => {
  const versions = ["0.1.0-alpha", "0.1.0-alpha.1", "0.1.0-alpha.beta", "0.1.0-beta", "0.1.0-beta.2", "0.1.0-beta.11", "0.1.0-next.4", "0.1.0-rc.1", "0.1.0", "0.1.1", "0.1.10", "0.2.0-next.1", "0.2.0", "1.0.0", "1.0.0+build.5", "10.0.0"];

  it("orders versions as semver does, prereleases below their release and build metadata ignored", () => {
    for (const a of versions) {
      for (const b of versions) expect(Math.sign(compareVersions(a, b)), `${a} vs ${b}`).toBe(semver.compare(a, b));
    }
    expect(() => compareVersions("0.1", "0.1.0")).toThrow("0.1 is not a semantic version");
  });

  it("moves next when it is unset or older, never when it is the version or newer", () => {
    expect(nextTrails(null, "0.1.4")).toBe(true);
    expect(nextTrails(undefined, "0.1.4")).toBe(true);
    expect(nextTrails("0.1.3", "0.1.4")).toBe(true);
    expect(nextTrails("0.1.0-next.4", "0.1.0")).toBe(true);
    expect(nextTrails("0.1.4", "0.1.4")).toBe(false);
    expect(nextTrails("0.1.5-next.1", "0.1.4")).toBe(false);
  });
});

describe("previewRefusal", () => {
  it("refuses a preview older than latest, and allows a newer one or one for a package with no latest", () => {
    expect(previewRefusal("0.1.0-next.5", "next", "0.1.0")).toBe("0.1.0-next.5 is older than latest 0.1.0: a next preview must not trail latest");
    expect(previewRefusal("0.0.9-beta.1", "beta", "0.1.0")).toBe("0.0.9-beta.1 is older than latest 0.1.0: a beta preview must not trail latest");
    expect(previewRefusal("0.1.1-next.1", "next", "0.1.0")).toBeNull();
    expect(previewRefusal("0.1.0-next.1", "next", undefined)).toBeNull();
  });
});

describe("raiseNext", () => {
  const run = async (next, version) => {
    const moved = [];
    const line = await raiseNext("@neralabs/rerail", version, { readNext: async () => next, distTag: (name, at, tag) => void moved.push(`${name}@${at} ${tag}`) });
    return { moved, line };
  };

  it("points a trailing next at the version and says what it was", async () => {
    expect(await run("0.1.0-next.4", "0.1.0")).toEqual({ moved: ["@neralabs/rerail@0.1.0 next"], line: "pointed next at @neralabs/rerail@0.1.0 (it was 0.1.0-next.4)" });
    expect(await run(null, "0.1.0")).toEqual({ moved: ["@neralabs/rerail@0.1.0 next"], line: "pointed next at @neralabs/rerail@0.1.0 (it was not set)" });
  });

  it("leaves a next already at the version, as a re-run finds it, or at a newer preview", async () => {
    expect(await run("0.1.0", "0.1.0")).toEqual({ moved: [], line: "@neralabs/rerail@0.1.0: next is 0.1.0, not older; left there" });
    expect((await run("0.1.1-next.1", "0.1.0")).moved).toEqual([]);
  });
});
