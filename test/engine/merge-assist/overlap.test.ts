import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { conflictId, quietFiles } from "../../../src/engine/merge-assist/conflicts.js";
import { contestedAnchor, identify, overlaps, ownBase, refHunks, workspaceRanges } from "../../../src/engine/merge-assist/overlap.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

const LINES = Array.from({ length: 40 }, (_, index) => `line ${index + 1}`);

/** A repository on `main` with `f.txt` of 40 lines, and how to change lines of it. */
interface Repository {
  readonly root: string;
  git(...args: string[]): string;
  write(changes: Readonly<Record<number, string>>): void;
}

function repository(): Repository {
  const root = mkdtempSync(join(tmpdir(), "rerail-overlap-"));
  roots.push(root);
  const git = (...args: string[]) => execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null", GIT_AUTHOR_NAME: "Ann", GIT_AUTHOR_EMAIL: "ann@example.com", GIT_COMMITTER_NAME: "Ann", GIT_COMMITTER_EMAIL: "ann@example.com" },
  }).trim();
  const write = (changes: Readonly<Record<number, string>>) => writeFileSync(join(root, "f.txt"), `${LINES.map((line, index) => changes[index + 1] ?? line).join("\n")}\n`);
  git("init", "-q", "-b", "main");
  write({});
  git("add", "f.txt");
  git("commit", "-q", "-m", "start");
  return { root, git, write };
}

/** A branch `theirs` from main changing `changes`, committed; the checkout is back on a branch `mine` from main. */
function theirBranch(repo: Repository, changes: Readonly<Record<number, string>>): string {
  repo.git("switch", "-q", "-c", "theirs");
  repo.write(changes);
  repo.git("commit", "-q", "-am", "Their change");
  const sha = repo.git("rev-parse", "HEAD");
  repo.git("switch", "-q", "-c", "mine", "main");
  return sha;
}

/** The files `mine`'s working changes overlap on `theirs`. */
function overlapping(repo: Repository, theirs: string): string[] {
  const base = repo.git("rev-parse", "main");
  const own = ownBase(repo.root, theirs, "refs/heads/main", base);
  if (own === null) return [];
  const ours = workspaceRanges(repo.root, base);
  return identify(ours, [{ ref: "theirs", files: refHunks(repo.root, base, own, theirs) }]).map(conflict => conflict.file);
}

describe("the overlap rule", () => {
  it("counts hunks at most 3 unchanged lines apart, and no further", () => {
    const at = (line: number) => ({ lo: line, hi: line, sig: `line ${line}` });
    expect(overlaps(at(10), at(10))).toBe(false);
    expect(overlaps(at(10), { lo: 10, hi: 10, sig: "other" })).toBe(true);
    expect(overlaps(at(10), at(14))).toBe(true);
    expect(overlaps(at(10), at(15))).toBe(false);

    const repo = repository();
    const theirs = theirBranch(repo, { 20: "theirs 20" });
    repo.write({ 24: "mine 24" });
    expect(overlapping(repo, theirs)).toEqual(["f.txt"]);
    repo.write({ 25: "mine 25" });
    expect(overlapping(repo, theirs)).toEqual([]);
  });

  it("does not count the same change made on both sides", () => {
    const repo = repository();
    const theirs = theirBranch(repo, { 20: "same 20" });
    repo.write({ 20: "same 20" });
    expect(overlapping(repo, theirs)).toEqual([]);
    repo.write({ 20: "different 20" });
    expect(overlapping(repo, theirs)).toEqual(["f.txt"]);
  });

  it("leaves out mainline work a ref merged in, and a ref mainline already holds", () => {
    const repo = repository();
    repo.git("switch", "-q", "-c", "theirs");
    repo.write({ 30: "theirs 30" });
    repo.git("commit", "-q", "-am", "Their change");
    repo.git("switch", "-q", "main");
    repo.write({ 10: "main 10" });
    repo.git("commit", "-q", "-am", "Mainline change");
    const base = repo.git("rev-parse", "HEAD");
    repo.git("switch", "-q", "theirs");
    repo.git("merge", "-q", "--no-edit", "main");
    const theirs = repo.git("rev-parse", "HEAD");
    repo.git("switch", "-q", "-c", "mine", "main");

    expect(ownBase(repo.root, theirs, "refs/heads/main", base)).toBe(base);
    // Line 10 is mainline's, merged into theirs: editing next to it overlaps nothing of theirs.
    repo.write({ 10: "main 10", 11: "mine 11" });
    expect(overlapping(repo, theirs)).toEqual([]);
    repo.write({ 10: "main 10", 31: "mine 31" });
    expect(overlapping(repo, theirs)).toEqual(["f.txt"]);

    // Once mainline holds all of theirs, it has no work of its own.
    repo.git("checkout", "-q", "--", "f.txt");
    repo.git("switch", "-q", "main");
    repo.git("merge", "-q", "--no-edit", "theirs");
    expect(ownBase(repo.root, theirs, "refs/heads/main", repo.git("rev-parse", "HEAD"))).toBeNull();
  });
});

describe("a conflict's quiet files and id", () => {
  it("keeps quiet only overlaps entirely in lockfiles, changelogs, generated output and docs", () => {
    for (const file of ["package-lock.json", "Cargo.lock", "web/yarn.lock", "CHANGELOG.md", "dist/app.js", "src/api.generated.ts", "docs/guide.md"]) {
      expect(quietFiles([file]), file).toBe(true);
    }
    expect(quietFiles(["docs/guide.md", "src/model.ts"])).toBe(false);
    expect(quietFiles(["README.md"])).toBe(false);
    expect(quietFiles([])).toBe(false);
  });

  it("keeps the same id for the same question and ref", () => {
    expect(conflictId("overlap:abc", "refs/pull/398/head")).toBe(conflictId("overlap:abc", "refs/pull/398/head"));
    expect(conflictId("overlap:abc", "refs/pull/398/head")).toMatch(/^conflict:overlap:abc:[0-9a-f]{8}$/u);
    expect(conflictId("overlap:abc", "jbarnes/x")).not.toBe(conflictId("overlap:abc", "refs/pull/398/head"));
  });

  it("anchors what a change contests on the base text: lines added above or whitespace keep it, other text changes it", () => {
    const repo = repository();
    const start = repo.git("rev-parse", "HEAD");
    const anchor = (base: string, lines: Array<{ lo: number; hi: number }>) => contestedAnchor(repo.root, base, "f.txt", lines);
    const contested = anchor(start, [{ lo: 20, hi: 21 }]);
    // Three lines added at the top move the contested lines to 23–24, and one of them is re-indented.
    const shifted = (changes: Readonly<Record<number, string>>) => {
      writeFileSync(join(repo.root, "f.txt"), `${["top 1", "top 2", "top 3", ...LINES.map((line, index) => changes[index + 1] ?? line)].join("\n")}\n`);
      repo.git("commit", "-q", "-am", "Lines above");
      return repo.git("rev-parse", "HEAD");
    };
    expect(anchor(shifted({ 20: "   line   20" }), [{ lo: 23, hi: 24 }])).toBe(contested);
    expect(anchor(shifted({ 21: "line twenty-one" }), [{ lo: 23, hi: 24 }])).not.toBe(contested);
    // An insertion after line 19 anchors on line 19; a file the base does not hold, on no text.
    expect(anchor(start, [{ lo: 19.5, hi: 19.5 }])).toBe(anchor(start, [{ lo: 19, hi: 19 }]));
    expect(contestedAnchor(repo.root, start, "missing.txt", [{ lo: 0.5, hi: 0.5 }])).toMatch(/^[0-9a-f]{24}$/u);
    expect(anchor(start, [{ lo: 30, hi: 30 }])).not.toBe(contested);
  });
});
