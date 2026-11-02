// scripts/release/github-release.mjs: the GitHub Release each released version gets, its notes the version's changelog
// section, the gh call that makes it or rewrites its notes and flags, the remote release tags a backfill may read, and
// the Releases a backfill makes for release tags without one.

import { describe, expect, it } from "vitest";
import { NO_CHANGELOG_ENTRIES } from "../../scripts/release/core.mjs";
import { backfillPlan, backfillTags, changelogPath, ghReleaseArgs, githubRelease, parseRemoteTags, releasedByTag } from "../../scripts/release/github-release.mjs";

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

  it("rewrites the notes and both flags of a Release that exists, so a run made again, or a put-back, sets them right", () => {
    expect(ghReleaseArgs(release, true)).toEqual(["release", "edit", "v0.1.1", "--notes-file", "-", "--prerelease=false", "--latest=true"]);
    expect(ghReleaseArgs({ ...release, prerelease: true, latest: false }, true)).toEqual(["release", "edit", "v0.1.1", "--notes-file", "-", "--prerelease=true", "--latest=false"]);
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

describe("parseRemoteTags", () => {
  it("maps each tag to its commit, an annotated tag's from its peeled line, whichever order git lists them in", () => {
    const text = [
      "5282f738\trefs/tags/v0.1.0-next.1",
      "abf575c5\trefs/tags/v0.1.0-next.1^{}",
      "b9988b1e\trefs/tags/v0.1.0-next.3",
      `1234abcd\trefs/tags/${PYTHON}@0.1.4`,
      `1111aaaa\trefs/tags/${PYTHON}@0.1.4^{}`,
      "",
    ].join("\n");
    expect([...parseRemoteTags(text)]).toEqual([
      ["v0.1.0-next.1", "abf575c5"],
      ["v0.1.0-next.3", "b9988b1e"],
      [`${PYTHON}@0.1.4`, "1111aaaa"],
    ]);
    expect(parseRemoteTags("")).toEqual(new Map());
  });
});

describe("backfillTags", () => {
  const local = [
    { tag: "v0.1.0-next.0", commit: "aaaa" },
    { tag: "v0.1.0-next.1", commit: "bbbb" },
    { tag: "port-readiness-recordings", commit: "cccc" },
    { tag: "scratch", commit: "dddd" },
    { tag: `${PYTHON}@0.1.4`, commit: "eeee" },
    { tag: "v0.1.0", commit: "ffff" },
  ];
  const remote = new Map([
    ["v0.1.0-next.1", "b111"],
    [`${PYTHON}@0.1.4`, "eeee"],
    ["v0.1.0", "ffff"],
    ["v0.1.1", "0000"],
    ["gate-recordings", "9999"],
  ]);

  it("keeps, in local order, the release tags at the remote's commit, and names what it leaves out", () => {
    expect(backfillTags(local, remote)).toEqual({
      tags: [`${PYTHON}@0.1.4`, "v0.1.0"],
      localOnly: ["v0.1.0-next.0"],
      diverged: ["v0.1.0-next.1"],
      missing: ["v0.1.1"],
    });
  });

  it("leaves tags that mark no release out of every list", () => {
    const { localOnly, diverged, missing } = backfillTags(local, remote);
    for (const tag of ["port-readiness-recordings", "scratch", "gate-recordings"]) {
      expect([...localOnly, ...diverged, ...missing]).not.toContain(tag);
    }
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
    const { releases } = backfillPlan(tags, new Set(["port-readiness-recordings"]), readAt);
    expect(releases.map(({ tag }) => tag)).toEqual([`${PYTHON}@0.1.4`, "v0.1.0", "v0.1.1"]);
    expect(releases[0]).toMatchObject({ notes: "### Patch Changes\n\n- e06db22: Messages.", prerelease: false, latest: false });
    expect(releases[1]).toMatchObject({ notes: "First release.", latest: false });
    expect(releases[2]).toMatchObject({ notes: "- Fixed a thing.", latest: true });
  });

  it("leaves a tag whose changelog has no section for its version, or no changelog at all, without a Release, and names it", () => {
    const { releases, noNotes } = backfillPlan(tags, new Set(), readAt);
    expect(noNotes).toEqual(["v0.1.0-next.4", "@neralabs/rerail-pack-sql@0.1.1"]);
    expect(releases.some(({ notes }) => notes === NO_CHANGELOG_ENTRIES)).toBe(false);
  });

  it("does not name a tag that has its Release already, whatever its changelog says", () => {
    const { noNotes } = backfillPlan(tags, new Set(["v0.1.0-next.4"]), readAt);
    expect(noNotes).toEqual(["@neralabs/rerail-pack-sql@0.1.1"]);
  });

  it("marks no older core version latest when the newest has its Release already", () => {
    const { releases } = backfillPlan(tags, new Set(["v0.1.1", `${PYTHON}@0.1.4`]), readAt);
    expect(releases.map(({ tag }) => tag)).toEqual(["v0.1.0"]);
    expect(releases.some(({ latest }) => latest)).toBe(false);
  });
});
