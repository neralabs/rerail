// scripts/release/candidate/candidate.mjs: the release candidate that is tagged and the Release pull request that is merged,
// following main or frozen at a commit with fixes cherry-picked onto it. Each test drives runReleaseAction against a
// real git remote in a temporary folder, with a stand-in for release-version.mjs that bumps a version file and moves
// the pending .changelog/ entries into CHANGELOG.md, as the Release commit does.

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { actionComment, CANDIDATE_BRANCH, candidateCiVerdict, candidateMessage, candidateTrustRefusal, cherryPickRefusal, coreVersionRefusal, FLOW_MARKER, freezeRefusal, gitIn, parseTarget, pickedTree, pullRequestHeadRefusal, pullRequestMessage, pullRequestTree, releaseTrainRefusal, readReleaseMessage, RELEASE_BRANCH, releasedCommit, releaseTrainTag, runReleaseAction } from "../../scripts/release/candidate/candidate.mjs";
import { removeFolderSync } from "../support/platform/remove-folder.ts";

const SHA = "a".repeat(40);
const OTHER = "b".repeat(40);

describe("release commit messages", () => {
  it("round-trips a candidate's base and cherry-picks, and a frozen pull request's candidate", () => {
    expect(readReleaseMessage(candidateMessage({ base: SHA, picks: [OTHER] }))).toEqual({ base: SHA, picks: [OTHER], coreVersion: "" });
    expect(readReleaseMessage(candidateMessage({ base: SHA }))).toEqual({ base: SHA, picks: [], coreVersion: "" });
    expect(readReleaseMessage(candidateMessage({ base: SHA, coreVersion: "0.2.0" }))).toEqual({ base: SHA, picks: [], coreVersion: "0.2.0" });
    expect(readReleaseMessage(pullRequestMessage(SHA))).toEqual({ candidate: SHA });
  });

  it("reads no release from a commit made by hand or by Update branch", () => {
    expect(readReleaseMessage("Merge branch 'main' into changeset-release/main")).toBeNull();
    expect(readReleaseMessage("Release")).toBeNull();
    expect(readReleaseMessage(`Fix\n\nRelease-Base: ${SHA}`)).toBeNull();
    expect(readReleaseMessage("Release\n\nRelease-Base: main")).toBeNull();
  });
});

describe("parseTarget", () => {
  it("reads a commit, a release tag, a pull request number, or nothing", () => {
    expect(parseTarget("")).toBeNull();
    expect(parseTarget(" ABCDEF1 ")).toEqual({ rev: "abcdef1" });
    expect(parseTarget("v0.1.5")).toEqual({ rev: "v0.1.5" });
    expect(parseTarget("729")).toEqual({ pull: 729 });
    expect(parseTarget("#729")).toEqual({ pull: 729 });
  });

  it("refuses a branch name or anything else git would resolve loosely", () => {
    for (const text of ["main", "HEAD~1", "abc", "v0.1.5-next.1", "origin/main"]) expect(() => parseTarget(text), text).toThrow(/is not a commit/u);
  });
});

describe("refusals", () => {
  it("freezes a commit on main or the latest release, never one older than the freeze workflow", () => {
    expect(freezeRefusal({ commit: SHA, onMain: true, latestRelease: false, hasFlow: true })).toBeNull();
    expect(freezeRefusal({ commit: SHA, onMain: false, latestRelease: true, hasFlow: true })).toBeNull();
    expect(freezeRefusal({ commit: SHA, onMain: false, latestRelease: false, hasFlow: true })).toMatch(/not on main/u);
    expect(freezeRefusal({ commit: SHA, onMain: true, latestRelease: false, hasFlow: false })).toMatch(/older than the freeze workflow/u);
  });

  it("cherry-picks only a commit on main that a frozen release lacks", () => {
    expect(cherryPickRefusal({ commit: SHA, frozen: true, onMain: true, included: false })).toBeNull();
    expect(cherryPickRefusal({ commit: SHA, frozen: false, onMain: true, included: false })).toMatch(/freeze action first/u);
    expect(cherryPickRefusal({ commit: SHA, frozen: true, onMain: false, included: false })).toMatch(/Merge the fix to main first/u);
    expect(cherryPickRefusal({ commit: SHA, frozen: true, onMain: true, included: true })).toMatch(/already in/u);
  });
});

describe("coreVersionRefusal", () => {
  it("takes a plain version later than the latest release", () => {
    expect(coreVersionRefusal("0.2.0", "0.1.5")).toBeNull();
    expect(coreVersionRefusal("1.0.0", null)).toBeNull();
    for (const version of ["0.2", "v0.2.0", "0.2.0-next.1", "minor", ""]) expect(coreVersionRefusal(version, "0.1.5"), version).toMatch(/is not a version like 0\.2\.0/u);
    expect(coreVersionRefusal("0.1.5", "0.1.5")).toMatch(/0\.1\.5 is not later than the latest release, 0\.1\.5/u);
    expect(coreVersionRefusal("0.1.4", "0.1.5")).toMatch(/not later than/u);
  });
});

describe("candidateCiVerdict", () => {
  it("reads ci.yml's newest run on the candidate", () => {
    const run = (created_at, status, conclusion = null) => ({ created_at, status, conclusion });
    expect(candidateCiVerdict([])).toBe("pending");
    expect(candidateCiVerdict([run("2026-01-01T00:00:00Z", "in_progress")])).toBe("pending");
    expect(candidateCiVerdict([run("2026-01-01T00:00:00Z", "completed", "success")])).toBe("passed");
    expect(candidateCiVerdict([run("2026-01-01T00:00:00Z", "completed", "success"), run("2026-01-02T00:00:00Z", "completed", "failure")])).toBe("failed");
    expect(candidateCiVerdict([run("2026-01-02T00:00:00Z", "in_progress"), run("2026-01-01T00:00:00Z", "completed", "success")])).toBe("pending");
  });
});

describe("releasedCommit", () => {
  it("is the candidate a frozen head names, the head itself when it followed main, and nothing for any other head", () => {
    expect(releasedCommit(SHA, pullRequestMessage(OTHER))).toBe(OTHER);
    expect(releasedCommit(SHA, candidateMessage({ base: OTHER }))).toBe(SHA);
    expect(releasedCommit(SHA, "Release")).toBeNull();
    expect(releasedCommit(SHA, "Merge branch 'main' into changeset-release/main")).toBeNull();
  });
});

describe("pullRequestHeadRefusal", () => {
  const following = { named: { base: OTHER, picks: [] }, parents: [OTHER], parentOnMain: true, version: "0.2.0" };
  const frozen = { named: { candidate: SHA }, parents: [OTHER], parentOnMain: true, version: "0.2.0" };
  const candidate = { tip: SHA, named: { base: OTHER, picks: [OTHER] }, commits: 2, version: "0.2.0" };

  it("passes the head release-pr.yml built, following main or frozen", () => {
    expect(pullRequestHeadRefusal({ head: following })).toBeNull();
    expect(pullRequestHeadRefusal({ head: frozen, candidate })).toBeNull();
  });

  it("refuses a head changed by hand or by Update branch, saying how to rebuild it", () => {
    expect(pullRequestHeadRefusal({ head: { ...following, named: null } })).toMatch(/not a commit release-pr\.yml made.*action=freeze/su);
    expect(pullRequestHeadRefusal({ head: { ...following, parents: [OTHER, SHA] } })).toMatch(/does not sit on a commit of main/u);
    expect(pullRequestHeadRefusal({ head: { ...following, parentOnMain: false } })).toMatch(/does not sit on a commit of main/u);
    expect(pullRequestHeadRefusal({ head: { ...following, parents: [SHA] } })).toMatch(/names bbbbbbbbb as its base but sits on aaaaaaaaa/u);
  });

  it("refuses a frozen head whose candidate moved, was pushed to, or is another version", () => {
    expect(pullRequestHeadRefusal({ head: frozen, candidate: { tip: null } })).toMatch(/is gone/u);
    expect(pullRequestHeadRefusal({ head: frozen, candidate: { tip: OTHER } })).toMatch(/is at bbbbbbbbb/u);
    expect(pullRequestHeadRefusal({ head: frozen, candidate: { ...candidate, commits: 3 } })).toMatch(/pushed to by hand/u);
    expect(pullRequestHeadRefusal({ head: frozen, candidate: { ...candidate, named: null } })).toMatch(/pushed to by hand/u);
    expect(pullRequestHeadRefusal({ head: frozen, candidate: { ...candidate, version: "0.1.9" } })).toMatch(/is version 0\.1\.9, but this pull request sets 0\.2\.0/u);
  });
});

describe("actionComment", () => {
  const frozen = { releasable: true, changed: true, frozen: true, candidate: SHA, base: OTHER, picks: [SHA], dropped: [] };

  it("says what the release now is and where its builds are", () => {
    const comment = actionComment({ action: "cherry-pick", actor: "someone", repository: "owner/name", result: frozen });
    expect(comment).toContain("Fix cherry-picked by @someone. The release is now candidate `aaaaaaaaa`: base `bbbbbbbbb` plus `aaaaaaaaa`.");
    expect(comment).toContain("https://github.com/owner/name/actions/workflows/release.yml?query=branch%3Achangeset-release%2Fcandidate");
    expect(actionComment({ action: "unfreeze", actor: "someone", repository: "owner/name", result: { releasable: true, changed: true, frozen: false } })).toContain("follows `main` again");
  });

  it("says which version core was set to, frozen or following main", () => {
    expect(actionComment({ action: "set-version", actor: "someone", repository: "o/n", result: { releasable: true, changed: true, frozen: false, coreVersion: "0.2.0" } })).toBe("Core's version set to 0.2.0 by @someone. The checklist was cleared, because what merging releases changed.");
    expect(actionComment({ action: "set-version", actor: "someone", repository: "o/n", result: { releasable: true, changed: true, frozen: false } })).toMatch(/^Core's version set back to the next one by @someone\./u);
    expect(actionComment({ action: "set-version", actor: "someone", repository: "o/n", result: { ...frozen, coreVersion: "0.2.0" } })).toMatch(/^Core's version set to 0\.2\.0 by @someone\. The release is now candidate `aaaaaaaaa`/u);
  });

  it("says nothing for a push to main, unless main changed an entry the frozen release ships", () => {
    expect(actionComment({ action: "update", actor: "x", repository: "o/n", result: { ...frozen, changed: false } })).toBeNull();
    expect(actionComment({ action: "update", actor: "x", repository: "o/n", result: { releasable: true, changed: true, frozen: false } })).toBeNull();
    expect(actionComment({ action: "unfreeze", actor: "x", repository: "o/n", result: { releasable: true, changed: false, frozen: false } })).toBeNull();
    expect(actionComment({ action: "update", actor: "x", repository: "o/n", result: { ...frozen, changed: false, dropped: [".changelog/fixed-a.md"] } })).toMatch(/ships them as frozen.*`\.changelog\/fixed-a\.md`/su);
  });
});

describe("candidateTrustRefusal", () => {
  const trusted = { candidate: SHA, baseTrusted: true, picksOnMain: true, picksMatch: true, releaseMatches: true };

  it("passes a candidate that rebuilds from reviewed commits to what the pull request shows", () => {
    expect(candidateTrustRefusal(trusted)).toBeNull();
  });

  it("refuses one whose base, cherry-picks or Release commit are not what release-pr.yml builds", () => {
    expect(candidateTrustRefusal({ ...trusted, baseTrusted: false })).toMatch(/base is not a commit on main or the latest release.*action=freeze/su);
    expect(candidateTrustRefusal({ ...trusted, picksOnMain: false })).toMatch(/cherry-pick that is not a commit on main/u);
    expect(candidateTrustRefusal({ ...trusted, picksMatch: false })).toMatch(/changes that are not its base and the cherry-picks/u);
    expect(candidateTrustRefusal({ ...trusted, releaseMatches: false })).toMatch(/Release commit does something this pull request's changes do not show/u);
  });
});

describe("releaseTrainRefusal", () => {
  it("passes the tag release-pr.yml makes, at a commit a merged Release pull request released", () => {
    expect(releaseTrainRefusal({ tag: releaseTrainTag(SHA), commit: SHA, released: [OTHER, SHA] })).toBeNull();
  });

  it("refuses a tag named for another commit, or at a commit no Release pull request released", () => {
    expect(releaseTrainRefusal({ tag: "release-by-hand", commit: SHA, released: [SHA] })).toMatch(/is not the release tag of its commit aaaaaaaaa/u);
    expect(releaseTrainRefusal({ tag: releaseTrainTag(SHA), commit: SHA, released: [OTHER] })).toMatch(/not a commit a merged Release pull request released/u);
  });
});

describe("releaseTrainTag", () => {
  it("names one tag per released commit", () => {
    expect(releaseTrainTag(`0123456789abcdef${"0".repeat(24)}`)).toBe("release-0123456789ab");
  });
});

// Each test runs dozens of git commands against a real remote: seconds, more beside other test files or on Windows.
describe("runReleaseAction", { timeout: 120_000 }, () => {
  const folders = [];
  afterEach(() => {
    for (const folder of folders.splice(0)) removeFolderSync(folder);
  });

  /** A remote with a main branch, and a clone to commit to main from (`work`) beside the clean one actions run in (`ci`). */
  function repository() {
    const root = mkdtempSync(join(tmpdir(), "rerail-release-candidate-"));
    folders.push(root);
    const remote = join(root, "remote.git");
    execFileSync("git", ["init", "--quiet", "--bare", "--initial-branch=main", remote]);
    const clone = (name) => {
      const folder = join(root, name);
      execFileSync("git", ["clone", "--quiet", remote, folder], { stdio: "ignore" });
      const git = gitIn(folder);
      git(["config", "user.name", "Test"]);
      git(["config", "user.email", "test@example.com"]);
      git(["config", "commit.gpgsign", "false"]);
      // The stand-in reads and writes the files' text as committed, on Windows too.
      git(["config", "core.autocrlf", "false"]);
      return { folder, git };
    };
    const work = clone("work");
    work.git(["checkout", "--quiet", "-b", "main"]);
    /** Commits `files` (path to text, null to delete) to main and pushes; returns the commit. */
    const commit = (subject, files) => {
      for (const [path, text] of Object.entries(files)) {
        const file = join(work.folder, path);
        if (text === null) rmSync(file);
        else {
          mkdirSync(dirname(file), { recursive: true });
          writeFileSync(file, text);
        }
      }
      work.git(["add", "--all"]);
      work.git(["commit", "--quiet", "--message", subject]);
      work.git(["push", "--quiet", "origin", "HEAD:main"]);
      return work.git(["rev-parse", "HEAD"]);
    };
    commit("Start", { "version.txt": "1\n", "CHANGELOG.md": "# Changelog\n\n## 1\n\nFirst.\n", [FLOW_MARKER]: "// the freeze workflow\n", "src/app.txt": "app\n" });
    const ci = clone("ci");
    /**
     * release-version.mjs's stand-in: with entries pending, the next version (the one set by hand, when one is) and
     * their text in CHANGELOG.md. Returns the version in the checkout afterwards, as release-candidate.mjs's does.
     */
    const version = (frozen, coreVersion = "") => {
      const current = readFileSync(join(ci.folder, "version.txt"), "utf8").trim();
      const entries = join(ci.folder, ".changelog");
      const pending = existsSync(entries) ? readdirSync(entries).sort() : [];
      if (pending.length === 0) return current;
      const next = coreVersion === "" ? String(Number(current) + 1) : coreVersion;
      const notes = pending.map((name) => `- ${readFileSync(join(entries, name), "utf8").trim()}`);
      const changelog = readFileSync(join(ci.folder, "CHANGELOG.md"), "utf8").replace("# Changelog\n", `# Changelog\n\n## ${next}${frozen === null ? "" : " (frozen)"}\n\n${notes.join("\n")}\n`);
      writeFileSync(join(ci.folder, "CHANGELOG.md"), changelog);
      writeFileSync(join(ci.folder, "version.txt"), `${next}\n`);
      for (const name of pending) rmSync(join(entries, name));
      return next;
    };
    const run = (action, target = "", resolvePull = () => "", fixes = "", coreVersion = "") => runReleaseAction({ action, target, fixes, coreVersion, git: ci.git, remote: "origin", version, resolvePull });
    const remoteGit = gitIn(remote);
    const tip = (branch) => {
      try {
        return remoteGit(["rev-parse", "--verify", "--quiet", `refs/heads/${branch}`]);
      } catch {
        return null;
      }
    };
    const show = (rev, path) => remoteGit(["show", `${rev}:${path}`]);
    const has = (rev, path) => remoteGit(["ls-tree", "-r", "--name-only", rev]).split("\n").includes(path);
    /** Squash-merges the Release pull request into main, as GitHub does; returns main's new tip. */
    const merge = () => {
      work.git(["fetch", "--quiet", "origin"]);
      work.git(["reset", "--quiet", "--hard", "origin/main"]);
      work.git(["merge", "--quiet", "--squash", `origin/${RELEASE_BRANCH}`]);
      work.git(["commit", "--quiet", "--message", "Release (#1)"]);
      work.git(["push", "--quiet", "origin", "HEAD:main"]);
      return work.git(["rev-parse", "HEAD"]);
    };
    return { commit, run, tip, show, has, merge, remoteGit, work, ci, version };
  }

  it("follows main until frozen: each push rebuilds the Release commit on main's tip", () => {
    const { commit, run, tip, show, remoteGit } = repository();
    expect(run("update")).toEqual({ releasable: false, changed: false, frozen: false });
    expect(tip(RELEASE_BRANCH)).toBeNull();

    const first = commit("Add a", { ".changelog/added-a.md": "A.\n" });
    expect(run("update")).toEqual({ releasable: true, changed: true, frozen: false });
    expect(readReleaseMessage(remoteGit(["log", "-1", "--format=%B", tip(RELEASE_BRANCH)]))).toEqual({ base: first, picks: [], coreVersion: "" });
    expect(tip(CANDIDATE_BRANCH)).toBeNull();

    const second = commit("Add b", { ".changelog/added-b.md": "B.\n" });
    run("update");
    expect(remoteGit(["rev-parse", `${tip(RELEASE_BRANCH)}^`])).toBe(second);
    expect(show(tip(RELEASE_BRANCH), "CHANGELOG.md")).toContain("## 2\n\n- A.\n- B.\n");
  });

  it("frozen, pushes to main change neither the candidate nor the pull request", () => {
    const { commit, run, tip, has } = repository();
    const base = commit("Add a", { ".changelog/added-a.md": "A.\n" });
    const frozen = run("freeze");
    expect(frozen).toMatchObject({ releasable: true, changed: true, frozen: true, base, picks: [], dropped: [] });
    expect(tip(CANDIDATE_BRANCH)).toBe(frozen.candidate);
    const head = tip(RELEASE_BRANCH);

    commit("Later work", { "src/later.txt": "later\n", ".changelog/added-later.md": "Later.\n" });
    expect(run("update")).toMatchObject({ releasable: true, changed: false, frozen: true, candidate: frozen.candidate });
    expect(tip(CANDIDATE_BRANCH)).toBe(frozen.candidate);
    expect(tip(RELEASE_BRANCH)).toBe(head);
    expect(has(frozen.candidate, "src/later.txt")).toBe(false);
  });

  it("releases the frozen commit and its cherry-picks alone, and the merge leaves main only what was not released", () => {
    const { commit, run, tip, show, has, merge } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    const frozen = run("freeze");

    const reworded = commit("Reword a", { ".changelog/added-a.md": "A, reworded.\n" });
    commit("Unrelated work", { "src/unrelated.txt": "unrelated\n", ".changelog/added-unrelated.md": "Unrelated.\n" });
    const fix = commit("Fix the app", { "src/app.txt": "app, fixed\n", ".changelog/fixed-app.md": "Fixed.\n" });

    run("cherry-pick", reworded);
    const picked = run("cherry-pick", fix);
    expect(picked).toMatchObject({ changed: true, frozen: true, base: frozen.base, picks: [reworded, fix], dropped: [] });

    // The candidate is the tested tree: the base, the two cherry-picks, and the Release commit.
    const candidate = tip(CANDIDATE_BRANCH);
    expect(candidate).toBe(picked.candidate);
    expect(show(candidate, "src/app.txt")).toBe("app, fixed");
    expect(has(candidate, "src/unrelated.txt")).toBe(false);
    expect(show(candidate, "version.txt")).toBe("2");
    expect(show(candidate, "CHANGELOG.md")).toContain("## 2 (frozen)\n\n- A, reworded.\n- Fixed.\n");
    expect(show(candidate, "CHANGELOG.md")).not.toContain("Unrelated");

    // Merging the Release pull request is clean, and main keeps the entry the release did not ship.
    const merged = merge();
    expect(show(merged, "version.txt")).toBe("2");
    expect(show(merged, "CHANGELOG.md")).toContain("## 2 (frozen)\n\n- A, reworded.\n- Fixed.\n");
    expect(has(merged, ".changelog/added-a.md")).toBe(false);
    expect(has(merged, ".changelog/fixed-app.md")).toBe(false);
    expect(has(merged, ".changelog/added-unrelated.md")).toBe(true);
    expect(show(merged, "src/unrelated.txt")).toBe("unrelated");
  });

  it("cherry-picks a merged pull request by its number", () => {
    const { commit, run } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("freeze");
    const fix = commit("Fix the app", { "src/app.txt": "app, fixed\n" });
    const asked = [];
    expect(run("cherry-pick", "#42", (number) => (asked.push(number), fix))).toMatchObject({ picks: [fix] });
    expect(asked).toEqual([42]);
  });

  it("keeps cherry-picks in main's order, whatever order they were added in", () => {
    const { commit, run, show, tip } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("freeze");
    const first = commit("First fix", { "src/app.txt": "app, first\n" });
    const second = commit("Second fix", { "src/app.txt": "app, first, second\n" });
    // The second alone does not apply: it builds on the first.
    expect(() => run("cherry-pick", second)).toThrow(/does not apply to the frozen release.*Nothing was changed/su);
    run("cherry-pick", first);
    expect(run("cherry-pick", second)).toMatchObject({ picks: [first, second] });
    expect(show(tip(CANDIDATE_BRANCH), "src/app.txt")).toBe("app, first, second");
  });

  it("frozen, an entry main reworded without a cherry-pick ships as frozen and is gone after the merge", () => {
    const { commit, run, tip, show, has, merge } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    const frozen = run("freeze");
    const head = tip(RELEASE_BRANCH);
    commit("Reword a", { ".changelog/added-a.md": "A, reworded on main only.\n" });

    const updated = run("update");
    expect(updated).toMatchObject({ changed: false, frozen: true, candidate: frozen.candidate, dropped: [".changelog/added-a.md"] });
    expect(tip(RELEASE_BRANCH)).not.toBe(head);
    expect(tip(CANDIDATE_BRANCH)).toBe(frozen.candidate);

    const merged = merge();
    expect(has(merged, ".changelog/added-a.md")).toBe(false);
    expect(show(merged, "CHANGELOG.md")).toContain("- A.\n");
  });

  it("frozen, a change on main to the lines the release rewrites leaves the release whole and names the files", () => {
    const { commit, run, tip, merge } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    const frozen = run("freeze");
    const head = tip(RELEASE_BRANCH);
    const lead = commit("Edit the changelog by hand", { "CHANGELOG.md": "# Changelog\n\nA new lead.\n\n## 1\n\nFirst.\n" });

    const updated = run("update");
    expect(updated).toMatchObject({ releasable: true, changed: false, frozen: true, candidate: frozen.candidate, conflicts: ["CHANGELOG.md"] });
    expect(tip(RELEASE_BRANCH)).toBe(head);
    expect(tip(CANDIDATE_BRANCH)).toBe(frozen.candidate);
    const comment = actionComment({ action: "update", actor: "x", repository: "o/n", result: updated });
    expect(comment.split("\n")[0]).toBe(`<!-- release-conflict ${frozen.candidate} CHANGELOG.md -->`);
    expect(comment).toMatch(/can't be merged until this is settled.*`CHANGELOG\.md`.*action=cherry-pick/su);

    // Cherry-picking the commit that changed them settles it: the merge is clean again.
    run("cherry-pick", lead);
    expect(run("update").conflicts).toBeUndefined();
    merge();
  });

  it("a first freeze that meets such a change says to name it as a fix, since nothing is frozen to cherry-pick onto", () => {
    const { commit, run, tip } = repository();
    const base = commit("Add a", { ".changelog/added-a.md": "A.\n" });
    const lead = commit("Edit the changelog by hand", { "CHANGELOG.md": "# Changelog\n\nA new lead.\n\n## 1\n\nFirst.\n" });
    expect(() => run("freeze", base)).toThrow(/main changed `CHANGELOG\.md` after the frozen commit.*fixes input, or freeze a newer commit\. Nothing was changed/su);
    expect([tip(CANDIDATE_BRANCH), tip(RELEASE_BRANCH)]).toEqual([null, null]);
    expect(run("freeze", base, undefined, lead)).toMatchObject({ base, picks: [lead] });
  });

  it("refuses what would release something untested, and changes nothing", () => {
    const { commit, run, tip, work, ci } = repository();
    const base = commit("Add a", { ".changelog/added-a.md": "A.\n" });
    const fix = commit("Fix the app", { "src/app.txt": "app, fixed\n" });
    expect(() => run("cherry-pick", fix)).toThrow(/No release is frozen/u);

    work.git(["checkout", "--quiet", "-b", "side", base]);
    writeFileSync(join(work.folder, "side.txt"), "side\n");
    work.git(["add", "--all"]);
    work.git(["commit", "--quiet", "--message", "Side"]);
    work.git(["push", "--quiet", "origin", "side"]);
    const side = work.git(["rev-parse", "HEAD"]);
    work.git(["checkout", "--quiet", "main"]);
    // release-pr.yml's checkout holds every branch; the action itself fetches only main and the release branches.
    expect(() => run("freeze", side)).toThrow(/is not a commit this checkout has/u);
    ci.git(["fetch", "--quiet", "origin"]);
    expect(() => run("freeze", side)).toThrow(/not on main/u);
    expect(() => run("freeze", base, undefined, side)).toThrow(/not on main/u);
    expect(() => run("freeze", fix, undefined, base)).toThrow(/already in/u);

    run("freeze", base);
    const [candidate, head] = [tip(CANDIDATE_BRANCH), tip(RELEASE_BRANCH)];
    expect(() => run("cherry-pick", side)).toThrow(/not on main/u);
    expect(() => run("cherry-pick", base)).toThrow(/already in the frozen release/u);
    expect(() => run("cherry-pick")).toThrow(/needs the commit/u);
    expect(() => run("cherry-pick", "0000000")).toThrow(/is not a commit this checkout has/u);
    expect(() => run("merge")).toThrow(/unknown action/u);
    expect([tip(CANDIDATE_BRANCH), tip(RELEASE_BRANCH)]).toEqual([candidate, head]);
  });

  it("refuses to freeze a commit with nothing releasable, or one older than the freeze workflow", () => {
    const { commit, run, tip, remoteGit } = repository();
    expect(() => run("freeze")).toThrow(/nothing is releasable/u);
    expect(tip(CANDIDATE_BRANCH)).toBeNull();
    const old = commit("Remove the workflow", { [FLOW_MARKER]: null });
    commit("Restore it, with an entry", { [FLOW_MARKER]: "// the freeze workflow\n", ".changelog/added-a.md": "A.\n" });
    expect(() => run("freeze", old)).toThrow(/older than the freeze workflow/u);
    expect(remoteGit(["branch", "--list", "changeset-release/*"])).toBe("");
  });

  it("freezing again moves the base and keeps the cherry-picks it does not hold", () => {
    const { commit, run, has, tip } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("freeze");
    const middle = commit("Middle work", { "src/middle.txt": "middle\n" });
    const fix = commit("Fix the app", { "src/app.txt": "app, fixed\n" });
    run("cherry-pick", fix);

    expect(run("freeze", middle)).toMatchObject({ base: middle, picks: [fix] });
    expect(has(tip(CANDIDATE_BRANCH), "src/middle.txt")).toBe(true);
    expect(run("freeze")).toMatchObject({ base: fix, picks: [] });
  });

  it("unfreezes back to following main, and removes the candidate branch", () => {
    const { commit, run, tip, remoteGit } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    expect(run("unfreeze")).toMatchObject({ changed: false, frozen: false, notice: expect.stringMatching(/No release is frozen/u) });
    run("freeze");
    const later = commit("Later work", { "src/later.txt": "later\n" });

    expect(run("unfreeze")).toEqual({ releasable: true, changed: true, frozen: false });
    expect(tip(CANDIDATE_BRANCH)).toBeNull();
    expect(readReleaseMessage(remoteGit(["log", "-1", "--format=%B", tip(RELEASE_BRANCH)]))).toEqual({ base: later, picks: [], coreVersion: "" });
  });

  it("ships a fix on top of the latest release without the rest of main", () => {
    const { commit, run, tip, show, has, merge, remoteGit } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    const released = run("freeze").candidate;
    merge();
    remoteGit(["tag", "v0.0.2", released]);
    run("update");

    commit("Unfinished work", { "src/unfinished.txt": "unfinished\n", ".changelog/added-unfinished.md": "Unfinished.\n" });
    const fix = commit("Fix the app", { "src/app.txt": "app, fixed\n", ".changelog/fixed-app.md": "Fixed.\n" });
    expect(() => run("freeze", "v0.0.2")).toThrow(/nothing is releasable.*fixes input/su);
    const hotfix = run("freeze", "v0.0.2", (number) => (number === 7 ? fix : ""), "#7");
    expect(hotfix).toMatchObject({ base: released, picks: [fix] });
    expect(has(hotfix.candidate, "src/unfinished.txt")).toBe(false);
    expect(show(hotfix.candidate, "version.txt")).toBe("3");
    expect(show(hotfix.candidate, "CHANGELOG.md")).toContain("## 3 (frozen)\n\n- Fixed.\n");

    const merged = merge();
    expect(show(merged, "version.txt")).toBe("3");
    expect(has(merged, ".changelog/fixed-app.md")).toBe(false);
    expect(has(merged, ".changelog/added-unfinished.md")).toBe(true);
    expect(tip(CANDIDATE_BRANCH)).toBe(hotfix.candidate);
  });

  it("refuses a fix an earlier release already cherry-picked, so its release note never ships twice", () => {
    const { commit, run, remoteGit, merge } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    const released = run("freeze").candidate;
    merge();
    remoteGit(["tag", "v0.0.2", released]);
    const fix = commit("Fix the app", { "src/app.txt": "app, fixed\n", ".changelog/fixed-app.md": "Fixed.\n" });
    const first = run("freeze", "v0.0.2", undefined, fix).candidate;
    merge();
    remoteGit(["tag", "v0.0.3", first]);
    // The fix is on main and in v0.0.3, whose commit main's history does not reach.
    const again = commit("Fix it again", { "src/app.txt": "app, fixed twice\n", ".changelog/fixed-again.md": "Fixed again.\n" });
    expect(() => run("freeze", "v0.0.3", undefined, `${fix} ${again}`)).toThrow(/already in the frozen release/u);
    expect(run("freeze", "v0.0.3", undefined, again)).toMatchObject({ base: first, picks: [again] });
    expect(() => run("cherry-pick", fix)).toThrow(/already in the frozen release/u);
  });

  it("releases core as the version set by hand, following main, until it is set back", () => {
    const { commit, run, tip, show, remoteGit } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("update");
    expect(show(tip(RELEASE_BRANCH), "version.txt")).toBe("2");

    expect(run("set-version", "", undefined, "", "0.2.0")).toEqual({ releasable: true, changed: true, frozen: false, coreVersion: "0.2.0" });
    expect(show(tip(RELEASE_BRANCH), "version.txt")).toBe("0.2.0");
    expect(show(tip(RELEASE_BRANCH), "CHANGELOG.md")).toContain("## 0.2.0\n\n- A.\n");

    // A push to main rebuilds the release, and keeps the version.
    const later = commit("Add b", { ".changelog/added-b.md": "B.\n" });
    expect(run("update")).toEqual({ releasable: true, changed: true, frozen: false, coreVersion: "0.2.0" });
    expect(show(tip(RELEASE_BRANCH), "version.txt")).toBe("0.2.0");
    expect(readReleaseMessage(remoteGit(["log", "-1", "--format=%B", tip(RELEASE_BRANCH)]))).toEqual({ base: later, picks: [], coreVersion: "0.2.0" });

    expect(run("set-version")).toEqual({ releasable: true, changed: true, frozen: false });
    expect(show(tip(RELEASE_BRANCH), "version.txt")).toBe("2");
  });

  it("releases a frozen release as the version set by hand, through cherry-picks and an unfreeze", () => {
    const { commit, run, tip, show } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    const frozen = run("freeze");
    const set = run("set-version", "", undefined, "", "0.2.0");
    expect(set).toMatchObject({ changed: true, frozen: true, base: frozen.base, picks: [], coreVersion: "0.2.0" });
    expect(show(set.candidate, "version.txt")).toBe("0.2.0");
    expect(show(tip(RELEASE_BRANCH), "version.txt")).toBe("0.2.0");

    const fix = commit("Fix the app", { "src/app.txt": "app, fixed\n" });
    const picked = run("cherry-pick", fix);
    expect(picked).toMatchObject({ picks: [fix], coreVersion: "0.2.0" });
    expect(show(picked.candidate, "version.txt")).toBe("0.2.0");
    expect(run("update")).toMatchObject({ changed: false, coreVersion: "0.2.0" });

    expect(run("unfreeze")).toMatchObject({ frozen: false, coreVersion: "0.2.0" });
    expect(show(tip(RELEASE_BRANCH), "version.txt")).toBe("0.2.0");
    // Freezing with a version does both in one step.
    expect(run("freeze", "", undefined, "", "0.3.0")).toMatchObject({ frozen: true, coreVersion: "0.3.0" });
    expect(show(tip(CANDIDATE_BRANCH), "version.txt")).toBe("0.3.0");
  });

  it("refuses a version that is no version, not later than the latest release, or for a release core is not part of", () => {
    const { commit, run, tip, remoteGit } = repository();
    expect(() => run("set-version", "", undefined, "", "0.2.0")).toThrow(/core is not released as 0\.2\.0.*Nothing was changed/su);
    expect(tip(RELEASE_BRANCH)).toBeNull();

    const base = commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("update");
    const head = tip(RELEASE_BRANCH);
    expect(() => run("set-version", "", undefined, "", "0.2")).toThrow(/is not a version like 0\.2\.0.*Nothing was changed/su);
    remoteGit(["tag", "v0.3.0", base]);
    expect(() => run("set-version", "", undefined, "", "0.2.0")).toThrow(/0\.2\.0 is not later than the latest release, 0\.3\.0/u);
    expect(() => run("freeze", "", undefined, "", "0.3.0")).toThrow(/not later than the latest release/u);
    expect([tip(RELEASE_BRANCH), tip(CANDIDATE_BRANCH)]).toEqual([head, null]);
    expect(run("set-version", "", undefined, "", "0.3.1")).toMatchObject({ coreVersion: "0.3.1" });
  });

  it("a frozen candidate rebuilds to the trees it was built with, and one changed by hand does not", () => {
    const { commit, run, tip, ci } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("freeze");
    const fix = commit("Fix the app", { "src/app.txt": "app, fixed\n", ".changelog/fixed-app.md": "Fixed.\n" });
    commit("Later work", { "src/later.txt": "later\n" });
    const { candidate, base, picks } = run("cherry-pick", fix);
    const head = tip(RELEASE_BRANCH);
    const tree = (rev) => ci.git(["rev-parse", `${rev}^{tree}`]);
    const parent = ci.git(["rev-parse", `${head}^`]);
    expect(pickedTree(ci.git, base, picks)).toBe(tree(`${candidate}^`));
    expect(pullRequestTree(ci.git, candidate, parent)).toBe(tree(head));

    // A change slipped in under the Release commit: the candidate is no longer its base and cherry-picks.
    ci.git(["checkout", "--quiet", "--force", "--detach", `${candidate}^`]);
    writeFileSync(join(ci.folder, "src/app.txt"), "app, fixed, and more\n");
    ci.git(["commit", "--quiet", "--all", "--message", "Slipped in"]);
    ci.git(["cherry-pick", candidate]);
    const slipped = ci.git(["rev-parse", "HEAD"]);
    expect(pickedTree(ci.git, base, picks)).not.toBe(tree(`${slipped}^`));

    // A Release commit that also changes code: the pull request's changes do not show it.
    ci.git(["checkout", "--quiet", "--force", "--detach", candidate]);
    writeFileSync(join(ci.folder, "src/app.txt"), "app, fixed, and more\n");
    ci.git(["commit", "--quiet", "--all", "--amend", "--no-edit"]);
    const amended = ci.git(["rev-parse", "HEAD"]);
    expect(pullRequestTree(ci.git, amended, parent)).not.toBe(tree(head));
    ci.git(["checkout", "--quiet", "--force", "--detach", "refs/release/main"]);
  });

  it("an update that loses the race to a freeze leaves the frozen release alone", () => {
    const { commit, run, tip, ci, remoteGit } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("update");
    // A freeze lands between the update reading the branch and pushing it: the remote's branch moves under it.
    const push = ci.git;
    let raced = false;
    const racing = (args, options) => {
      if (args[0] === "push" && !raced) {
        raced = true;
        remoteGit(["update-ref", `refs/heads/${RELEASE_BRANCH}`, remoteGit(["rev-parse", "refs/heads/main"])]);
      }
      return push(args, options);
    };
    commit("Add b", { ".changelog/added-b.md": "B.\n" });
    const moved = () => remoteGit(["rev-parse", "refs/heads/main"]);
    const result = runReleaseAction({ action: "update", git: racing, remote: "origin", version: () => writeFileSync(join(ci.folder, "version.txt"), "9\n"), resolvePull: () => "" });
    expect(result).toMatchObject({ changed: false, frozen: false, notice: expect.stringMatching(/changed while this run was building/u) });
    expect(tip(RELEASE_BRANCH)).toBe(moved());
  });

  /** `ci.git`, with `land` run once before each of the first `times` pushes: another run landing while this one builds. */
  const racing = (ci, land, times = 1) => {
    let raced = 0;
    const git = (args, options) => {
      if (args[0] === "push" && raced < times) {
        raced += 1;
        land();
      }
      return ci.git(args, options);
    };
    return { git, raced: () => raced };
  };

  it("cherry-picks run at once both land: the one that pushes second builds again on the first", () => {
    const { commit, run, ci, version } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("freeze");
    const first = commit("First fix", { "src/first.txt": "first\n" });
    const second = commit("Second fix", { "src/second.txt": "second\n" });
    // The cherry-pick of the first fix lands while the second's run is building.
    const race = racing(ci, () => run("cherry-pick", first));
    const picked = runReleaseAction({ action: "cherry-pick", target: second, git: race.git, remote: "origin", version, resolvePull: () => "" });
    expect(race.raced()).toBe(1);
    expect(picked).toMatchObject({ changed: true, frozen: true, picks: [first, second] });
  });

  it("set-version that loses the race to a push's run builds again and sets the version, and refuses once it keeps losing", () => {
    const { commit, run, tip, show, ci, version, remoteGit } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("update");
    // Each push's run lands a commit of its own.
    let runs = 0;
    const pushRun = () => {
      runs += 1;
      const landed = remoteGit(["-c", "user.name=Test", "-c", "user.email=test@example.com", "commit-tree", "refs/heads/main^{tree}", "-p", "refs/heads/main", "-m", `A push's run ${runs}`]);
      remoteGit(["update-ref", `refs/heads/${RELEASE_BRANCH}`, landed]);
    };
    const setVersion = (git) => runReleaseAction({ action: "set-version", coreVersion: "0.2.0", git, remote: "origin", version, resolvePull: () => "" });

    const once = racing(ci, pushRun);
    expect(setVersion(once.git)).toEqual({ releasable: true, changed: true, frozen: false, coreVersion: "0.2.0" });
    expect(once.raced()).toBe(1);
    expect(show(tip(RELEASE_BRANCH), "version.txt")).toBe("0.2.0");

    const always = racing(ci, pushRun, Number.POSITIVE_INFINITY);
    expect(() => setVersion(always.git)).toThrow(/changed 3 times while this ran.*run set-version again/u);
    expect(always.raced()).toBe(3);
  });

  it("never builds on a Release pull request's branch that outlived its merge, nor keeps its version", () => {
    const { commit, run, tip, ci, version, merge, remoteGit } = repository();
    commit("Add a", { ".changelog/added-a.md": "A.\n" });
    run("freeze", "", undefined, "", "0.2.0");
    const released = tip(RELEASE_BRANCH);
    merge();
    // Both branches are still there: the repository keeps merged branches, and the tag job did not run.
    expect(tip(CANDIDATE_BRANCH)).not.toBeNull();
    const later = commit("Add b", { ".changelog/added-b.md": "B.\n" });
    const result = runReleaseAction({ action: "update", git: ci.git, remote: "origin", version, resolvePull: () => "", mergedHead: released });
    expect(result).toEqual({ releasable: true, changed: true, frozen: false });
    expect(readReleaseMessage(remoteGit(["log", "-1", "--format=%B", tip(RELEASE_BRANCH)]))).toMatchObject({ base: later, picks: [] });
  });

  it("refuses to run in a checkout with changes, so it never discards anyone's work", () => {
    const { run, ci } = repository();
    writeFileSync(join(ci.folder, "draft.md"), "draft\n");
    expect(() => run("update")).toThrow(/the checkout has changes/u);
    expect(readFileSync(join(ci.folder, "draft.md"), "utf8")).toBe("draft\n");
  });
});
