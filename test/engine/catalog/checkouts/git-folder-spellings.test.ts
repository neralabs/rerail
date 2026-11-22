import { execFileSync, spawnSync } from "node:child_process";
import { randomInt } from "node:crypto";
import { lstatSync, mkdirSync, realpathSync, unlinkSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { isDeepStrictEqual } from "node:util";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { storedForm } from "../../../../src/engine/catalog/checkout-control.js";
import { gitFolderReached } from "../../../../src/engine/catalog/checkouts/git-placement.js";
import { repositoryIdentityLocation } from "../../../../src/engine/catalog/repository-identity.js";
import { linkFolder } from "../../../support/platform/links.js";
import { removeFolder, removeFolderSync } from "../../../support/platform/remove-folder.js";

const WINDOWS = process.platform === "win32";

/** How many spellings a run compares, each asked from two paths with the main checkout there and deleted. */
const SPELLING_COUNT = 520;

/** This run's seed: `RERAIL_SPELLINGS_SEED` repeats a run's spellings, on any machine; without it each run draws one. */
const SEED = Number(process.env["RERAIL_SPELLINGS_SEED"] ?? randomInt(2 ** 31));

/** A small seeded generator (mulberry32): one seed gives the same numbers on every machine. */
function numbers(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let mixed = Math.imul(state ^ (state >>> 15), state | 1);
    mixed ^= mixed + Math.imul(mixed ^ (mixed >>> 7), mixed | 61);
    return ((mixed ^ (mixed >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * The segments a spelling is built of, from the case's root: names (`repos`, `sub`, `wts`), `.`, `..`, links to folders
 * (`lm` to `repos`, `hop` to `repos/sub`), and `dead`, a link whose folder is gone. Two names are drawn twice as often,
 * so more spellings reach the main checkout.
 */
const SEGMENTS = ["repos", "repos", "sub", "wts", ".", "..", "lm", "lm", "hop", "dead"] as const;

/** The names after the segments: the worktree's Git folder in the main checkout, mostly written clean. */
const TAILS: readonly { readonly names: readonly string[]; readonly clean: boolean }[] = [
  { names: ["main", ".git", "worktrees", "feature"], clean: true },
  { names: ["main", ".git", "worktrees", "feature"], clean: true },
  { names: ["main", ".git", "worktrees", "feature"], clean: true },
  { names: ["main", ".git", "worktrees", "feature", "."], clean: false },
  { names: ["main", ".git", "worktrees", "feature", ""], clean: false },
  { names: ["main", "", ".git", "worktrees", "feature"], clean: false },
];

interface Spelled {
  readonly gitdir: string;
  /** Whether it is one of the two forms Git writes, read from the segments it was built of. */
  readonly answered: boolean;
}

function pick<T>(next: () => number, items: readonly T[]): T {
  return items[Math.floor(next() * items.length)]!;
}

/** One spelling: absolute from the root (on Windows at times with no drive letter) or relative from the worktree. */
function spelled(next: () => number, root: string): Spelled {
  const absolute = next() < 0.5;
  const rooted = WINDOWS && absolute && next() < 0.25;
  const climbs = absolute ? [] : Array.from({ length: Math.floor(next() * 4) }, () => "..");
  const middle = Array.from({ length: Math.floor(next() * 4) }, () => pick(next, SEGMENTS));
  const tail = pick(next, TAILS);
  const separator = WINDOWS && next() < 0.5 ? "\\" : "/";
  const start = absolute ? [rooted ? root.slice(2) : root] : [];
  // A relative path climbs by its leading `..`; past them, and anywhere in an absolute path, a `.` or `..` is no form Git writes.
  const names = middle.slice(absolute ? 0 : leadingClimbs(middle));
  const answered = !rooted && tail.clean && names.every(segment => segment !== "." && segment !== "..");
  return { gitdir: [...start, ...climbs, ...middle, ...tail.names].join(separator), answered };
}

function leadingClimbs(segments: readonly string[]): number {
  const name = segments.findIndex(segment => segment !== "..");
  return name === -1 ? segments.length : name;
}

/** {@link SPELLING_COUNT} spellings, none twice, drawn from {@link SEED}. */
function spellings(root: string): readonly Spelled[] {
  const next = numbers(SEED);
  const drawn = new Map<string, Spelled>();
  for (let draw = 0; draw < 50 * SPELLING_COUNT && drawn.size < SPELLING_COUNT; draw += 1) {
    const one = spelled(next, root);
    drawn.set(one.gitdir, one);
  }
  return [...drawn.values()];
}

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** Git's own answer, asked from `folder` as spelled: its exit, and where it places the folder. */
function gitsAnswer(folder: string): { readonly status: number | null; readonly gitDir?: string; readonly top?: string; readonly common?: string } {
  const args = ["-C", folder, "rev-parse", "--path-format=absolute", "--git-dir", "--show-toplevel", "--git-common-dir"];
  const result = spawnSync("git", args, { encoding: "utf8" });
  if (result.status !== 0) return { status: result.status };
  const [gitDir, top, common] = result.stdout.trim().split("\n").map(line => resolve(line.trim()));
  return { status: 0, gitDir: gitDir!, top: top!, common: common! };
}

function missing(path: string): boolean {
  return lstatSync(path, { throwIfNoEntry: false }) === undefined;
}

/** Rerail's answer for `asked`, whose `.git` file holds `one`, against Git's; undefined where they are equal. */
function mismatch(asked: string, one: Spelled): string | undefined {
  const answer = gitsAnswer(asked);
  const placed = answer.status === 0;
  const want = {
    git: placed,
    workspaceRoot: placed ? answer.top : resolve(asked),
    common: placed ? storedForm(answer.common!) : undefined,
    gitFolderGone: one.answered && !placed ? true : undefined,
    reached: one.answered ? (placed ? storedForm(answer.gitDir!) : "gone") : undefined,
  };
  const location = repositoryIdentityLocation(asked);
  const reached = gitFolderReached(asked, one.gitdir);
  const got = {
    git: location.git,
    workspaceRoot: location.workspaceRoot,
    common: location.git ? storedForm(dirname(location.identityPath)) : undefined,
    gitFolderGone: location.gitFolderGone,
    reached: reached === undefined ? undefined : (missing(reached) ? "gone" : storedForm(reached)),
  };
  return isDeepStrictEqual(want, got) ? undefined : JSON.stringify({ seed: SEED, asked, gitdir: one.gitdir, status: answer.status, want, got });
}

let root = "";
let worktree = "";

/** Every spelling written into the worktree's `.git` file in turn, asked by its own path and through a link. */
function mismatches(): { readonly asked: number; readonly found: readonly string[] } {
  const found: string[] = [];
  const all = spellings(root);
  for (const one of all) {
    // Written anew: Windows refuses to open a hidden file to replace it, and Git for Windows hides the one it writes.
    unlinkSync(join(worktree, ".git"));
    writeFileSync(join(worktree, ".git"), `gitdir: ${one.gitdir}\n`);
    for (const asked of [worktree, join(root, "links", "x", "current")]) found.push(...[mismatch(asked, one)].filter(text => text !== undefined));
  }
  return { asked: 2 * all.length, found };
}

beforeAll(async () => {
  root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-git-folder-spellings-")));
  const main = join(root, "repos", "main");
  mkdirSync(main, { recursive: true });
  mkdirSync(join(root, "repos", "sub"));
  git(main, "init", "-q");
  git(main, "commit", "-q", "--allow-empty", "-m", "base");
  worktree = join(root, "wts", "feature");
  git(main, "worktree", "add", "-q", "-b", "feature", worktree);
  linkFolder(join(root, "repos"), join(root, "lm"));
  linkFolder(join(root, "repos", "sub"), join(root, "hop"));
  mkdirSync(join(root, "gone"));
  linkFolder(join(root, "gone"), join(root, "dead"));
  removeFolderSync(join(root, "gone"));
  mkdirSync(join(root, "links", "x"), { recursive: true });
  linkFolder(worktree, join(root, "links", "x", "current"));
});

afterAll(async () => {
  if (root !== "") await removeFolder(root);
});

// The two cases run in order: the second deletes the main checkout the first placed worktrees in.
describe(`generated spellings of a .git file's path: Rerail's answer is Git's (seed ${SEED}, RERAIL_SPELLINGS_SEED repeats it)`, () => {
  it("with the main checkout there", { timeout: 1_200_000 }, () => {
    const { asked, found } = mismatches();
    expect(asked).toBeGreaterThanOrEqual(2 * 500);
    expect(found).toEqual([]);
  });

  it("with the main checkout deleted", { timeout: 1_200_000 }, () => {
    removeFolderSync(join(root, "repos", "main"));
    const { asked, found } = mismatches();
    expect(asked).toBeGreaterThanOrEqual(2 * 500);
    expect(found).toEqual([]);
  });
});
