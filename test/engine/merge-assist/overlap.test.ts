import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { removeFolderSync } from "../../support/platform/remove-folder.js";
import { conflictId, quietFiles } from "../../../src/engine/merge-assist/conflicts.js";
import { contestedAnchor, identify, OverlapReader, overlaps, workspaceRanges } from "../../../src/engine/merge-assist/overlap.js";
import { git as gitIn } from "../../support/merge-assist/git.js";

const roots: string[] = [];
afterEach(() => {
  for (const root of roots.splice(0)) removeFolderSync(root);
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
  const git = (...args: string[]) => gitIn(root, ...args);
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

/** A detection pass's reader for `mine`'s working changes, from `start` (main by default), with main's tip as the base. */
async function reader(repo: Repository, start = repo.git("rev-parse", "main")): Promise<{ reader: OverlapReader; ours: Awaited<ReturnType<typeof workspaceRanges>> }> {
  const ours = await workspaceRanges(repo.root, start);
  return { reader: new OverlapReader(repo.root, { start, baseTip: repo.git("rev-parse", "main") }, new Set(ours.keys())), ours };
}

/** The files `mine`'s working changes overlap on `theirs`, and the hunks of theirs that overlap, as `start` lines. */
async function overlapping(repo: Repository, theirs: string, start?: string): Promise<string[]> {
  const { reader: read, ours } = await reader(repo, start);
  const own = await read.ownBase(theirs);
  if (own === null) return [];
  return identify(ours, [{ ref: "theirs", files: await read.refHunks(own, theirs) }]).map(conflict => conflict.file);
}

describe("the overlap rule", () => {
  it("counts hunks at most 3 unchanged lines apart, and no further", async () => {
    const at = (line: number) => ({ lo: line, hi: line, sig: `line ${line}` });
    expect(overlaps(at(10), at(10))).toBe(false);
    expect(overlaps(at(10), { lo: 10, hi: 10, sig: "other" })).toBe(true);
    expect(overlaps(at(10), at(14))).toBe(true);
    expect(overlaps(at(10), at(15))).toBe(false);

    const repo = repository();
    const theirs = theirBranch(repo, { 20: "theirs 20" });
    repo.write({ 24: "mine 24" });
    expect(await overlapping(repo, theirs)).toEqual(["f.txt"]);
    repo.write({ 25: "mine 25" });
    expect(await overlapping(repo, theirs)).toEqual([]);
  });

  it("does not count the same change made on both sides", async () => {
    const repo = repository();
    const theirs = theirBranch(repo, { 20: "same 20" });
    repo.write({ 20: "same 20" });
    expect(await overlapping(repo, theirs)).toEqual([]);
    repo.write({ 20: "different 20" });
    expect(await overlapping(repo, theirs)).toEqual(["f.txt"]);
  });

  it("leaves out mainline work a ref merged in, and a ref mainline already holds", async () => {
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

    expect(await (await reader(repo, base)).reader.ownBase(theirs)).toBe(base);
    // Line 10 is mainline's, merged into theirs: editing next to it overlaps nothing of theirs.
    repo.write({ 10: "main 10", 11: "mine 11" });
    expect(await overlapping(repo, theirs)).toEqual([]);
    repo.write({ 10: "main 10", 31: "mine 31" });
    expect(await overlapping(repo, theirs)).toEqual(["f.txt"]);

    // Once mainline holds all of theirs, it has no work of its own.
    repo.git("checkout", "-q", "--", "f.txt");
    repo.git("switch", "-q", "main");
    repo.git("merge", "-q", "--no-edit", "theirs");
    expect(await (await reader(repo)).reader.ownBase(theirs)).toBeNull();
  });
});

// Each test builds a repository with real Git commits and merges, which takes seconds on a busy machine.
describe("work the base already holds, and a ref that forked elsewhere", { timeout: 30_000 }, () => {
  it("leaves out a branch squash-merged into the base, also after the base moved on, near its lines too", async () => {
    const repo = repository();
    const theirs = theirBranch(repo, { 20: "theirs 20", 21: "theirs 21" });
    const start = repo.git("rev-parse", "main");
    repo.write({ 22: "mine 22" });
    expect(await overlapping(repo, theirs)).toEqual(["f.txt"]);

    // The branch's work lands on main as one new commit; the branch stays behind, not an ancestor of main.
    const squash = (changes: Readonly<Record<number, string>>, message: string) => {
      repo.git("stash", "-q");
      repo.git("switch", "-q", "main");
      repo.write(changes);
      repo.git("commit", "-q", "-am", message);
      repo.git("switch", "-q", "mine");
      repo.git("stash", "pop", "-q");
    };
    squash({ 20: "theirs 20", 21: "theirs 21" }, "Their change (#12)");
    expect(await overlapping(repo, theirs, start)).toEqual([]);
    // Main moved on elsewhere, then right next to the squashed lines.
    squash({ 5: "main 5", 20: "theirs 20", 21: "theirs 21" }, "Elsewhere");
    expect(await overlapping(repo, theirs, start)).toEqual([]);
    squash({ 5: "main 5", 19: "main 19", 20: "theirs 20", 21: "theirs 21" }, "Next to it");
    expect(await overlapping(repo, theirs, start)).toEqual([]);
    // Another branch's lines merged in beside theirs.
    squash({ 5: "main 5", 19: "main 19", 20: "theirs 20", 21: "theirs 21\nother 21" }, "Beside it");
    expect(await overlapping(repo, theirs, start)).toEqual([]);
    // Main took only part of it: the rest is still the branch's live work.
    const partial = repository();
    const partly = theirBranch(partial, { 20: "theirs 20", 30: "theirs 30" });
    const partialStart = partial.git("rev-parse", "main");
    partial.git("switch", "-q", "main");
    partial.write({ 20: "theirs 20" });
    partial.git("commit", "-q", "-am", "Half of it");
    partial.git("switch", "-q", "mine");
    partial.write({ 22: "mine 22", 31: "mine 31" });
    const { reader: read, ours } = await reader(partial, partialStart);
    const own = await read.ownBase(partly);
    expect(identify(ours, [{ ref: "theirs", files: await read.refHunks(own!, partly) }]).map(conflict => conflict.theirs.map(hunk => hunk.lo)))
      .toEqual([[30]]);
  });

  it("carries a ref that forked before the base gained lines above to this side's lines", async () => {
    const repo = repository();
    const theirs = theirBranch(repo, { 20: "theirs 20" });
    const forked = repo.git("rev-parse", "main");
    repo.git("switch", "-q", "main");
    writeFileSync(join(repo.root, "f.txt"), `${["top 1", "top 2", "top 3", "top 4", "top 5", ...LINES].join("\n")}\n`);
    repo.git("commit", "-q", "-am", "Five lines on top");
    repo.git("switch", "-q", "-C", "mine", "main");
    const start = repo.git("rev-parse", "HEAD");
    const edit = (line: number, text: string) => writeFileSync(join(repo.root, "f.txt"), `${["top 1", "top 2", "top 3", "top 4", "top 5", ...LINES.map((original, index) => (index + 1 === line ? text : original))].join("\n")}\n`);

    const { reader: read } = await reader(repo, start);
    expect(await read.ownBase(theirs)).toBe(forked);
    // Their line 20 is this side's line 25.
    edit(22, "mine 22");
    const { reader: near, ours } = await reader(repo, start);
    const [conflict] = identify(ours, [{ ref: "theirs", files: await near.refHunks(forked, theirs) }]);
    expect(conflict?.theirs.map(hunk => [hunk.lo, hunk.hi])).toEqual([[25, 25]]);
    // Anchored on the same text at either base.
    expect(await near.anchor("f.txt", conflict!.theirs)).toBe(await contestedAnchor(repo.root, forked, "f.txt", [{ lo: 20, hi: 20 }]));
    edit(30, "mine 30");
    expect(await overlapping(repo, theirs, start)).toEqual([]);
  });

  it("reads files named like an object's own properties", async () => {
    const repo = repository();
    repo.git("switch", "-q", "-c", "theirs");
    for (const name of ["constructor", "__proto__", "toString"]) writeFileSync(join(repo.root, name), "theirs\n");
    repo.git("add", "-A");
    repo.git("commit", "-q", "-m", "Their files");
    const theirs = repo.git("rev-parse", "HEAD");
    repo.git("switch", "-q", "-c", "mine", "main");
    for (const name of ["constructor", "__proto__", "toString"]) writeFileSync(join(repo.root, name), "mine\n");
    expect((await overlapping(repo, theirs)).sort()).toEqual(["__proto__", "constructor", "toString"]);
  });
});

describe("a conflict's quiet files and id", () => {
  it("keeps quiet only overlaps entirely in lockfiles, changelogs, generated output and docs", () => {
    for (const file of ["package-lock.json", "Cargo.lock", "web/yarn.lock", "CHANGELOG.md", "dist/app.js", "src/api.generated.ts", "docs/guide.md"]) {
      expect(quietFiles([file]), file).toBe(true);
    }
    expect(quietFiles(["docs/guide.md", "src/model.ts"])).toBe(false);
    expect(quietFiles(["README.md"])).toBe(false);
    for (const file of ["HISTORY.md", "NEWS", "CHANGES.rst", "changelog.txt", "docs/News.adoc"]) expect(quietFiles([file]), file).toBe(true);
    // Source files that share a changelog's name are code.
    for (const file of ["src/history.ts", "components/News.tsx", "app/models/changes.rb", "src/changelog.ts"]) expect(quietFiles([file]), file).toBe(false);
    expect(quietFiles([])).toBe(false);
  });

  it("keeps the same id for the same question and ref", () => {
    expect(conflictId("overlap:abc", "refs/pull/398/head")).toBe(conflictId("overlap:abc", "refs/pull/398/head"));
    expect(conflictId("overlap:abc", "refs/pull/398/head")).toMatch(/^conflict:overlap:abc:[0-9a-f]{8}$/u);
    expect(conflictId("overlap:abc", "jbarnes/x")).not.toBe(conflictId("overlap:abc", "refs/pull/398/head"));
  });

  it("anchors what a change contests on the base text: lines added above or whitespace keep it, other text changes it", async () => {
    const repo = repository();
    const start = repo.git("rev-parse", "HEAD");
    const anchor = async (base: string, lines: Array<{ lo: number; hi: number }>) => await contestedAnchor(repo.root, base, "f.txt", lines);
    const contested = await anchor(start, [{ lo: 20, hi: 21 }]);
    // Three lines added at the top move the contested lines to 23–24, and one of them is re-indented.
    const shifted = (changes: Readonly<Record<number, string>>) => {
      writeFileSync(join(repo.root, "f.txt"), `${["top 1", "top 2", "top 3", ...LINES.map((line, index) => changes[index + 1] ?? line)].join("\n")}\n`);
      repo.git("commit", "-q", "-am", "Lines above");
      return repo.git("rev-parse", "HEAD");
    };
    expect(await anchor(shifted({ 20: "   line   20" }), [{ lo: 23, hi: 24 }])).toBe(contested);
    expect(await anchor(shifted({ 21: "line twenty-one" }), [{ lo: 23, hi: 24 }])).not.toBe(contested);
    // An insertion after line 19 anchors on line 19; a file the base does not hold, on no text.
    expect(await anchor(start, [{ lo: 19.5, hi: 19.5 }])).toBe(await anchor(start, [{ lo: 19, hi: 19 }]));
    expect(await contestedAnchor(repo.root, start, "missing.txt", [{ lo: 0.5, hi: 0.5 }])).toMatch(/^[0-9a-f]{24}$/u);
    expect(await anchor(start, [{ lo: 30, hi: 30 }])).not.toBe(contested);
  });
});
