// scripts/release/github-release.mjs: the GitHub Release each released version gets, its notes the version's changelog
// section, the gh call that makes it or rewrites its notes, and the Releases a backfill makes for release tags without
// one.

import { describe, expect, it } from "vitest";
import { NO_CHANGELOG_ENTRIES } from "../../scripts/release/core.mjs";
import { backfillPlan, changelogPath, ghReleaseArgs, githubRelease, releasedByTag } from "../../scripts/release/github-release.mjs";

const CORE = "@neralabs/rerail";
const PYTHON = "@neralabs/rerail-pack-python";

const coreChangelog = ["# Changelog", "", "## Unreleased", "", "- Coming.", "", "## 0.1.1 (2026-11-02)", "", "- Fixed a thing.", "", "## 0.1.0 (2026-10-27)", "", "First release.", ""].join("\n");
const packChangelog = [`# ${PYTHON}`, "", "## 0.1.5", "", "### Patch Changes", "", "- 7392aea: README notes.", "", "## 0.1.4", "", "### Patch Changes", "", "- e06db22: Messages.", ""].join("\n");

describe("changelogPath", () => {
  it("reads core's notes from the root changelog and a pack's from its own", () => {
    expect(changelogPath(CORE)).toBe("CHANGELOG.md");
    expect(changelogPath(PYTHON)).toBe("packs/python/CHANGELOG.md");
    expect(() => changelogPath("left-pad")).toThrow("neither core nor a rule pack");
  });
});

describe("githubRelease", () => {
  it("makes core's latest release the repository's latest, its notes the version's section", () => {
    expect(githubRelease({ name: CORE, version: "0.1.1", distTag: "latest" }, coreChangelog)).toEqual({
      tag: "v0.1.1",
      title: "v0.1.1",
      notes: "- Fixed a thing.",
      prerelease: false,
      latest: true,
    });
  });

  it("makes core's next and beta releases pre-releases, never the latest", () => {
    for (const distTag of ["next", "beta"]) {
      expect(githubRelease({ name: CORE, version: `0.2.0-${distTag}.1`, distTag }, coreChangelog)).toMatchObject({ tag: `v0.2.0-${distTag}.1`, prerelease: true, latest: false });
    }
  });

  it("says a version without a changelog section had no entries, as the Release pull request does", () => {
    expect(githubRelease({ name: CORE, version: "0.1.0-next.4", distTag: "next" }, coreChangelog).notes).toBe(NO_CHANGELOG_ENTRIES);
    expect(githubRelease({ name: CORE, version: "0.1.0", distTag: "latest" }, "").notes).toBe(NO_CHANGELOG_ENTRIES);
  });

  it("never makes a pack's release the latest, and makes it a pre-release only when held under next", () => {
    expect(githubRelease({ name: PYTHON, version: "0.1.4", distTag: "latest" }, packChangelog)).toEqual({
      tag: `${PYTHON}@0.1.4`,
      title: `${PYTHON}@0.1.4`,
      notes: "### Patch Changes\n\n- e06db22: Messages.",
      prerelease: false,
      latest: false,
    });
    expect(githubRelease({ name: PYTHON, version: "0.1.5", distTag: "next" }, packChangelog)).toMatchObject({ prerelease: true, latest: false });
  });
});

describe("ghReleaseArgs", () => {
  const release = { tag: "v0.1.1", title: "v0.1.1", notes: "- Fixed a thing.", prerelease: false, latest: true };

  it("creates a Release only on an existing tag, with its notes from stdin and every flag explicit", () => {
    expect(ghReleaseArgs(release, false)).toEqual(["release", "create", "v0.1.1", "--verify-tag", "--title", "v0.1.1", "--notes-file", "-", "--prerelease=false", "--latest=true"]);
    expect(ghReleaseArgs({ ...release, prerelease: true, latest: false }, false)).toContain("--latest=false");
  });

  it("rewrites only the notes of a Release that exists, so a run can be run again", () => {
    expect(ghReleaseArgs(release, true)).toEqual(["release", "edit", "v0.1.1", "--notes-file", "-"]);
  });
});

describe("releasedByTag", () => {
  it("reads core's channel from its tag, as release.yml does, and takes a pack's as latest", () => {
    expect(releasedByTag("v0.1.0")).toEqual({ name: CORE, version: "0.1.0", distTag: "latest" });
    expect(releasedByTag("v0.1.0-next.4")).toEqual({ name: CORE, version: "0.1.0-next.4", distTag: "next" });
    expect(releasedByTag("v0.2.0-beta.1")).toEqual({ name: CORE, version: "0.2.0-beta.1", distTag: "beta" });
    expect(releasedByTag(`${PYTHON}@0.1.5`)).toEqual({ name: PYTHON, version: "0.1.5", distTag: "latest" });
  });

  it("leaves out every other tag", () => {
    for (const tag of ["port-readiness-recordings", "gate-recordings", "v0.1.0-rc.1", "v1", "@neralabs/rerail@0.1.0", "@other/pkg@1.0.0"]) expect(releasedByTag(tag), tag).toBeNull();
  });
});

describe("backfillPlan", () => {
  const files = {
    "v0.1.0-next.4": { "CHANGELOG.md": "# Changelog\n\n## 0.1.0 (unreleased)\n\n- Coming.\n" },
    "v0.1.0": { "CHANGELOG.md": coreChangelog },
    "v0.1.1": { "CHANGELOG.md": coreChangelog },
    [`${PYTHON}@0.1.4`]: { "packs/python/CHANGELOG.md": packChangelog },
    "@neralabs/rerail-pack-sql@0.1.1": {},
  };
  const readAt = (tag, path) => files[tag]?.[path] ?? null;
  const tags = ["v0.1.0-next.4", "port-readiness-recordings", `${PYTHON}@0.1.4`, "@neralabs/rerail-pack-sql@0.1.1", "v0.1.0", "v0.1.1"];

  it("makes the Release of every release tag without one, in tag order, with the notes at that tag", () => {
    const plan = backfillPlan(tags, new Set(["port-readiness-recordings"]), readAt);
    expect(plan.map(({ tag }) => tag)).toEqual(["v0.1.0-next.4", `${PYTHON}@0.1.4`, "@neralabs/rerail-pack-sql@0.1.1", "v0.1.0", "v0.1.1"]);
    expect(plan[0]).toMatchObject({ notes: NO_CHANGELOG_ENTRIES, prerelease: true, latest: false });
    expect(plan[1]).toMatchObject({ notes: "### Patch Changes\n\n- e06db22: Messages.", prerelease: false, latest: false });
    expect(plan[2].notes).toBe(NO_CHANGELOG_ENTRIES);
    expect(plan[3]).toMatchObject({ notes: "First release.", latest: false });
    expect(plan[4]).toMatchObject({ notes: "- Fixed a thing.", latest: true });
  });

  it("marks no older core version latest when the newest has its Release already", () => {
    const plan = backfillPlan(tags, new Set(["v0.1.1", `${PYTHON}@0.1.4`]), readAt);
    expect(plan.map(({ tag }) => tag)).toEqual(["v0.1.0-next.4", "@neralabs/rerail-pack-sql@0.1.1", "v0.1.0"]);
    expect(plan.some(({ latest }) => latest)).toBe(false);
  });
});
