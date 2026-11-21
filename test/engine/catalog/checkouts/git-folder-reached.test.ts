import { execFileSync, spawnSync } from "node:child_process";
import { lstatSync, mkdirSync, realpathSync, unlinkSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi, type TestContext } from "vitest";
import { checkoutRealpath, storedForm } from "../../../../src/engine/catalog/checkout-control.js";
import { gitEntryFacts, gitFolderReached } from "../../../../src/engine/catalog/checkouts/git-placement.js";
import { repositoryIdentityLocation } from "../../../../src/engine/catalog/repository-identity.js";
import { recordCheckout } from "../../../../src/engine/catalog/repository-registry.js";
import { runGit } from "../../../../src/engine/git.js";
import { openRerailDatabase, type RerailDatabase } from "../../../../src/engine/storage/database.js";
import { linkFolder, symlinkOrSkip } from "../../../support/platform/links.js";
import { removeFolder, removeFolderSync } from "../../../support/platform/remove-folder.js";

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit) };
});

// Every real path read passes through, so a row can count what reaching its Git folder reads.
vi.mock("node:fs", async importOriginal => {
  const actual = await importOriginal<typeof import("node:fs")>();
  return { ...actual, realpathSync: Object.assign(vi.fn(actual.realpathSync), { native: vi.fn(actual.realpathSync.native) }) };
});

const roots: string[] = [];
const databases: RerailDatabase[] = [];
afterEach(async () => {
  for (const database of databases.splice(0)) database.$client.close();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

const WINDOWS = process.platform === "win32";
const G = join(".git", "worktrees", "feature");

/**
 * How a row spells the path its worktree's `.git` file holds, from the folder `root`: the main checkout is
 * `root/repos/main`, the worktree `root/wts/feature`, `root/hop` a link to `root/repos/sub`, and `m` in the worktree a
 * link to the main checkout. `flat`: on Windows, which takes a `..` off the text before it follows a link, the path
 * reaches `root/main` in place of `root/repos/main`. `reads`: the real paths reaching it reads, on Linux and macOS and
 * on Windows.
 */
interface Spelling {
  readonly name: string;
  readonly gitdir: (root: string) => string;
  readonly flat?: true;
  readonly reads: readonly [posix: number, windows: number];
}

const SPELLINGS: readonly Spelling[] = [
  { name: "absolute, clean", gitdir: root => join(root, "repos", "main", G), reads: [0, 0] },
  { name: "absolute, with a . name", gitdir: root => `${join(root, "repos")}/./main/${G}`, reads: [0, 0] },
  { name: "absolute, with link/..", gitdir: root => `${join(root, "hop")}/../main/${G}`, flat: true, reads: [1, 0] },
  { name: "relative, through a link in it", gitdir: () => `m/${G}`, reads: [0, 0] },
  { name: "relative", gitdir: () => `../../repos/main/${G}`, reads: [1, 1] },
  { name: "relative, with a . name", gitdir: () => `../../repos/main/${G}/.`, reads: [1, 1] },
  { name: "relative, with link/..", gitdir: () => `../../hop/../main/${G}`, flat: true, reads: [1, 1] },
];

/** How a row reaches the worktree: by its own path, or by `root/links/x/current`, a link to it. */
type Reach = "own path" | "link" | "directory link";

/** What a row does to the main checkout once the worktree is recorded. */
type After = "there" | "deleted" | "reverse";

interface Row {
  readonly spelling: Spelling;
  readonly reach: Reach;
  readonly after: After;
}

const REACHES: readonly Reach[] = WINDOWS ? ["own path", "link", "directory link"] : ["own path", "link"];

const ROWS: readonly (Row & { readonly name: string })[] = SPELLINGS.flatMap(spelling => [
  ...REACHES.flatMap(reach => (["there", "deleted"] as const).map(after => ({ spelling, reach, after }))),
  ...(spelling.flat === true ? [{ spelling, reach: "own path" as const, after: "reverse" as const }] : []),
]).map(row => ({ ...row, name: `${row.spelling.name}, by ${row.reach}, main checkout ${row.after}` }));

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/** Git's own answer, asked from `folder` as spelled: its Git folder and top level, or its exit where it refuses. */
function gitsAnswer(folder: string): { readonly status: number | null; readonly gitDir?: string; readonly top?: string } {
  const result = spawnSync("git", ["-C", folder, "rev-parse", "--path-format=absolute", "--git-dir", "--show-toplevel"], { encoding: "utf8" });
  if (result.status !== 0) return { status: result.status };
  const [gitDir, top] = result.stdout.trim().split("\n").map(line => resolve(line.trim()));
  return { status: 0, gitDir: gitDir!, top: top! };
}

function missing(path: string): boolean {
  return lstatSync(path, { throwIfNoEntry: false }) === undefined;
}

/** Links `path` to the folder `target` as the row reaches it: a symbolic link, a junction on Windows, or a directory link there. */
function linkAs(context: TestContext, reach: Reach, target: string, path: string): void {
  if (reach === "directory link") symlinkOrSkip(context, target, path, "dir");
  else linkFolder(target, path);
}

interface Made {
  readonly asked: string;
  readonly gitdir: string;
  /** The Git folder the row's path reaches, as the row's system reaches it. */
  readonly reaches: string;
}

/** The row's folders, its worktree's `.git` file holding the row's path, the folder it is asked from. */
function made(context: TestContext, root: string, row: Row): Made {
  const main = join(root, "repos", "main");
  mkdirSync(main, { recursive: true });
  mkdirSync(join(root, "repos", "sub"));
  git(main, "init", "-q");
  git(main, "commit", "-q", "--allow-empty", "-m", "base");
  const worktree = join(root, "wts", "feature");
  git(main, "worktree", "add", "-q", "-b", "feature", worktree);
  linkFolder(join(root, "repos", "sub"), join(root, "hop"));
  linkFolder(main, join(worktree, "m"));
  const gitdir = row.spelling.gitdir(root);
  // Written anew: Git for Windows marks the .git file hidden, and Windows refuses to open a hidden file to replace it.
  unlinkSync(join(worktree, ".git"));
  writeFileSync(join(worktree, ".git"), `gitdir: ${gitdir}\n`);
  mkdirSync(join(root, "links", "x"), { recursive: true });
  const asked = row.reach === "own path" ? worktree : join(root, "links", "x", "current");
  if (row.reach !== "own path") linkAs(context, row.reach, worktree, asked);
  return { asked, gitdir, reaches: join(root, ...(WINDOWS && row.spelling.flat === true ? [] : ["repos"]), "main", G) };
}

/** Deletes the main checkout Git reaches, and for the reverse row makes a folder where the path cleaned as text leads. */
function afterRecording(root: string, after: After): void {
  if (after === "there") return;
  removeFolderSync(join(root, "repos", "main"));
  if (after === "reverse") mkdirSync(join(root, "main", G), { recursive: true });
}

function realPathReads(): number {
  return vi.mocked(realpathSync).mock.calls.length + vi.mocked(realpathSync.native).mock.calls.length;
}

/** How many Git processes Rerail started to place `folder`. */
function locateCalls(folder: string): number {
  return vi.mocked(runGit).mock.calls.filter(([cwd, args]) => cwd === resolve(folder) && args.includes("--show-toplevel")).length;
}

describe("the Git folder Git reaches for a .git file's path, read once for every reader, as Git itself reaches it", () => {
  it.for(ROWS)("$name", async (row, context) => {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-git-folder-reached-")));
    roots.push(root);
    const { asked, gitdir, reaches } = made(context, root, row);
    const db = openRerailDatabase(join(root, "rerail.sqlite"));
    databases.push(db);
    const recorded = recordCheckout(db, asked);
    afterRecording(root, row.after);
    const answer = gitsAnswer(asked);
    const placed = answer.status === 0;
    const gone = !placed && missing(reaches);
    expect(placed ? storedForm(answer.gitDir!) : answer.status, "Git places the worktree where its Git folder is there").toBe(placed ? storedForm(reaches) : 128);

    // Every reader: the placement, the gone check and its `worktrees` name, and the facts an acceptance is kept under.
    const location = repositoryIdentityLocation(asked);
    const read = { git: location.git, workspaceRoot: location.workspaceRoot, gitFolderGone: location.gitFolderGone };
    expect(read).toEqual({ git: placed, workspaceRoot: placed ? answer.top : resolve(asked), gitFolderGone: gone ? true : undefined });
    if (placed) expect(gitEntryFacts(asked), "the facts hold the Git folder's real path").toContain(`\n${checkoutRealpath(answer.gitDir!)}\n`);
    if (placed) expect(askedAgain(asked), "Git processes a second placement starts").toBe(0);
    if (gone) expect(recordCheckout(db, asked).cloneId, "the recorded clone id is kept").toBe(recorded.cloneId);

    // The function against Git's answer, and the real paths it read to reach it.
    const before = realPathReads();
    const reached = gitFolderReached(asked, gitdir);
    expect(realPathReads() - before, "real paths read").toBe(row.spelling.reads[WINDOWS ? 1 : 0]);
    if (gone) expect(reached !== undefined && missing(reached), "the folder it reaches is gone").toBe(true);
    else expect(reached === undefined ? undefined : storedForm(reached)).toBe(storedForm(placed ? answer.gitDir! : reaches));
  });
});

/** How many Git processes a second placement of `folder` starts. */
function askedAgain(folder: string): number {
  const before = locateCalls(folder);
  expect(repositoryIdentityLocation(folder).git).toBe(true);
  return locateCalls(folder) - before;
}
