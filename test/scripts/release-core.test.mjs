// scripts/release/core.mjs, core-paths.mjs and dist-tags.mjs: when core is released (what it ships changed since its
// last tag), the version it takes, the changelog and version edits the Release pull request makes, the tag guard,
// `next` kept from trailing `latest`, and the check of a release's dist-tags.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import semver from "semver";
import { describe, expect, it } from "vitest";
import { coreChanges, lockChangesCore, matrixChanges, shipsWithCore } from "../../scripts/release/core-paths.mjs";
import { PLUGIN_PINS, changelogSection, coreTagRefusal, nextCoreVersion, releaseChangelog, setPluginVersion, setLockVersion, setPackageVersion } from "../../scripts/release/core.mjs";
import { compareVersions, distTagReport, nextTrails, parseDistTagList, parseExpectation, raiseNext } from "../../scripts/release/dist-tags.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..", "..");

describe("shipsWithCore", () => {
  it("counts the source, engine, build inputs and package files, and not the packs, docs, CI, tests or dev scripts", () => {
    for (const path of ["src/cli/cli.ts", "native/src/lib.rs", "languages/python.json", "ui/src/App.tsx", "package.json", "Cargo.lock", "scripts/build/copy-ui.mjs", "skills/rerail/SKILL.md", "README.md"]) {
      expect(shipsWithCore(path), path).toBe(true);
    }
    for (const path of ["packs/python/rules/x.json", "docs/pack-format.md", ".github/workflows/ci.yml", ".changeset/a.md", "test/scripts/x.test.mjs", "integration/x.test.ts", "scripts/release/core.mjs", "scripts/dev/rerail-dev.mjs", "CHANGELOG.md", "CONTRIBUTING.md"]) {
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
    expect(releaseChangelog("## [Unreleased]\n\n- a\n", "0.1.1", "2026-10-23")).toEqual({ text: "## 0.1.1 (2026-10-23)\n\n- a\n", notes: true });
  });
});

describe("releaseChangelog and changelogSection", () => {
  it("dates the unreleased section as the release, whatever its heading", () => {
    const named = releaseChangelog("# Changelog\n\n## 0.1.0 (unreleased)\n\n- a\n", "0.1.0", "2026-10-22");
    expect(named).toEqual({ text: "# Changelog\n\n## 0.1.0 (2026-10-22)\n\n- a\n", notes: true });
    const plain = releaseChangelog("# Changelog\n\n## Unreleased\n\n- b\n\n## 0.1.0 (2026-10-22)\n\n- a\n", "0.1.1", "2026-10-23");
    expect(plain.text).toBe("# Changelog\n\n## 0.1.1 (2026-10-23)\n\n- b\n\n## 0.1.0 (2026-10-22)\n\n- a\n");
  });

  it("adds a section saying there are no notes when nobody wrote an unreleased one", () => {
    const released = releaseChangelog("# Changelog\n\n## 0.1.0 (2026-10-22)\n\n- a\n", "0.1.1", "2026-10-23");
    expect(released).toEqual({ text: "# Changelog\n\n## 0.1.1 (2026-10-23)\n\nNo changelog entries were written for this release.\n\n## 0.1.0 (2026-10-22)\n\n- a\n", notes: false });
    expect(changelogSection(released.text, "0.1.1")).toBe("No changelog entries were written for this release.");
  });

  it("reads one version's section, dated as core's or bare as a pack's, and nothing for a version it lacks", () => {
    const text = "# Changelog\n\n## 0.1.4\n\n### Patch Changes\n\n- b\n\n## 0.1.3 (2026-10-01)\n\n- a\n";
    expect(changelogSection(text, "0.1.4")).toBe("### Patch Changes\n\n- b");
    expect(changelogSection(text, "0.1.3")).toBe("- a");
    expect(changelogSection(text, "0.1.2")).toBe("");
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
    const stale = distTagReport([expected[0]], () => ({ latest: "0.1.0-next.4" }));
    expect(stale.failures).toEqual(["@neralabs/rerail: latest is 0.1.0-next.4, not the 0.1.0 just published"]);
  });

  it("fails a package whose next is older than its latest, and passes one with no next or a newer one", () => {
    const trailing = distTagReport([expected[0]], () => ({ latest: "0.1.0", next: "0.1.0-next.4" }));
    expect(trailing.failures).toEqual(["@neralabs/rerail: next is 0.1.0-next.4, older than latest 0.1.0; a release under latest must move next up to it"]);
    expect(distTagReport([expected[0]], () => ({ latest: "0.1.0" })).failures).toEqual([]);
    expect(distTagReport([expected[0]], () => ({ latest: "0.1.0", next: "0.1.1-next.1" })).failures).toEqual([]);
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
