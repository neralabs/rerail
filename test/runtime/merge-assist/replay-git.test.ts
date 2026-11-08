import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { ownBase } from "../../../benchmark/merge-assist-replay.js";
import { identify, refFiles, refHunks } from "../../../src/engine/index.js";
import { statedIntent } from "../../../benchmark/merge-assist-replay-git.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** A repository whose commits are made at `at` (seconds after a fixed epoch), so first-parent walks by time are exact. */
function repository(): { root: string; git: (args: string[], at?: number) => string; write: (lines: string[]) => void } {
  const root = mkdtempSync(join(tmpdir(), "rerail-replay-git-"));
  roots.push(root);
  const git = (args: string[], at = 0) => execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    env: {
      ...process.env,
      GIT_CONFIG_NOSYSTEM: "1",
      GIT_CONFIG_GLOBAL: "/dev/null",
      GIT_AUTHOR_NAME: "Ann",
      GIT_AUTHOR_EMAIL: "ann@example.com",
      GIT_COMMITTER_NAME: "Ann",
      GIT_COMMITTER_EMAIL: "ann@example.com",
      GIT_AUTHOR_DATE: `${1_800_000_000 + at} +0000`,
      GIT_COMMITTER_DATE: `${1_800_000_000 + at} +0000`,
    },
  }).trim();
  git(["init", "-q", "-b", "main"]);
  return { root, git, write: lines => writeFileSync(join(root, "f.txt"), `${lines.join("\n")}\n`) };
}

const iso = (at: number) => new Date((1_800_000_000 + at) * 1000).toISOString();

describe("a ref's own work", () => {
  it("leaves out mainline work the ref merged in after the agent's base", () => {
    const { root, git, write } = repository();
    const lines = Array.from({ length: 40 }, (_, index) => `line ${index + 1}`);
    write(lines);
    git(["add", "f.txt"]);
    git(["commit", "-q", "-m", "start"], 0);
    const base = git(["rev-parse", "HEAD"]);
    git(["switch", "-q", "-c", "feature"]);
    write(lines.map((line, index) => (index === 29 ? "feature 30" : line)));
    git(["commit", "-q", "-am", "Change line 30"], 10);
    git(["switch", "-q", "main"]);
    write(lines.map((line, index) => (index === 9 ? "main 10" : line)));
    git(["commit", "-q", "-am", "Change line 10 on main"], 20);
    git(["switch", "-q", "feature"]);
    git(["merge", "-q", "--no-edit", "main"], 30);
    const ref = git(["rev-parse", "HEAD"]);

    const own = ownBase(root, ref, iso(40), "refs/heads/main", base);
    expect(own).toBe(git(["rev-parse", "main"]));
    expect(refFiles(root, own, ref)).toEqual(["f.txt"]);
    const hunks = refHunks(root, base, own, ref);
    // Only line 30 is the ref's own change; main's line 10 is not.
    expect(hunks["f.txt"]?.map(hunk => [hunk.lo, hunk.hi])).toEqual([[30, 30]]);
    expect(statedIntent(root, own, ref, null)).toBe("Change line 30");

    const side = { ref: "refs/heads/feature", files: hunks };
    const sig = "agent";
    expect(identify(new Map([["f.txt", [{ lo: 10, hi: 10, sig }]]]), [side])).toEqual([]);
    expect(identify(new Map([["f.txt", [{ lo: 31, hi: 31, sig }]]]), [side]).map(conflict => conflict.file)).toEqual(["f.txt"]);

    // The merge base with the agent's base, as before, would count main's line 10 as the ref's.
    expect(refHunks(root, base, base, ref)["f.txt"]?.map(hunk => [hunk.lo, hunk.hi])).toEqual([[10, 10], [30, 30]]);
  });

  it("measures against the base branch as it stood when the ref was visible", () => {
    const { root, git, write } = repository();
    const lines = Array.from({ length: 40 }, (_, index) => `line ${index + 1}`);
    write(lines);
    git(["add", "f.txt"]);
    git(["commit", "-q", "-m", "start"], 0);
    const base = git(["rev-parse", "HEAD"]);
    git(["switch", "-q", "-c", "feature"]);
    write(lines.map((line, index) => (index === 29 ? "feature 30" : line)));
    git(["commit", "-q", "-am", "Change line 30"], 10);
    const ref = git(["rev-parse", "HEAD"]);
    git(["switch", "-q", "main"]);
    git(["merge", "-q", "--no-ff", "--no-edit", "feature"], 50);
    // Before the merge the ref's own work starts at the base; after it, main holds all of it.
    expect(ownBase(root, ref, iso(20), "refs/heads/main", base)).toBe(base);
    expect(ownBase(root, ref, iso(60), "refs/heads/main", base)).toBe(ref);
    expect(ownBase(root, ref, iso(60), "refs/heads/main", base, base)).toBe(base);
  });

  it("leaves out the agent's own branch work a stacked ref carries", () => {
    const { root, git, write } = repository();
    const lines = Array.from({ length: 40 }, (_, index) => `line ${index + 1}`);
    write(lines);
    git(["add", "f.txt"]);
    git(["commit", "-q", "-m", "start"], 0);
    git(["switch", "-q", "-c", "agent"]);
    const agentLines = lines.map((line, index) => (index === 9 ? "agent 10" : line));
    write(agentLines);
    git(["commit", "-q", "-am", "Change line 10 on the agent's branch"], 10);
    const base = git(["rev-parse", "HEAD"]);
    git(["switch", "-q", "-c", "stacked"]);
    write(agentLines.map((line, index) => (index === 29 ? "stacked 30" : line)));
    git(["commit", "-q", "-am", "Change line 30 on the stacked branch"], 20);
    const ref = git(["rev-parse", "HEAD"]);

    // Against main the stacked branch would own line 10 too; the agent's base already holds that.
    const own = ownBase(root, ref, iso(30), "refs/heads/main", base);
    expect(own).toBe(base);
    expect(refHunks(root, base, own, ref)["f.txt"]?.map(hunk => [hunk.lo, hunk.hi])).toEqual([[30, 30]]);
  });
});
