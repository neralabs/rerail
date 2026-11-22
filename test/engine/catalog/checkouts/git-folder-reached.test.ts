import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, lstatSync, mkdirSync, realpathSync, unlinkSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi, type TestContext } from "vitest";
import { checkoutRealpath, storedForm } from "../../../../src/engine/catalog/checkout-control.js";
import { gitEntryFacts, gitFolderReached, writtenForm } from "../../../../src/engine/catalog/checkouts/git-placement.js";
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
const substituted: string[] = [];
afterEach(async () => {
  for (const database of databases.splice(0)) database.$client.close();
  for (const drive of substituted.splice(0)) spawnSync("subst", [drive, "/d"], { stdio: "ignore" });
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

const WINDOWS = process.platform === "win32";
const G = join(".git", "worktrees", "feature");

/**
 * How a row spells the path its worktree's `.git` file holds, from the folder `root`: the main checkout is
 * `root/repos/main`, the worktree `root/wts/feature`, `root/hop` a link to `root/repos/sub`, `root/dead` a link whose
 * folder is gone, and `m` in the worktree a link to the main checkout. `answered`: one of the two forms Git writes there,
 * which Rerail answers itself; every other spelling is left to Git. `flat`: on Windows, which takes a `..` off the text
 * before it follows a link, the path reaches `root/main` in place of `root/repos/main`. `reads`: the real paths reaching
 * it reads.
 */
interface Spelling {
  readonly name: string;
  readonly gitdir: (root: string) => string;
  readonly answered: boolean;
  readonly flat?: true;
  readonly reads: number;
}

const SPELLINGS: readonly Spelling[] = [
  { name: "absolute, clean", gitdir: root => join(root, "repos", "main", G), answered: true, reads: 0 },
  { name: "relative", gitdir: () => `../../repos/main/${G}`, answered: true, reads: 1 },
  { name: "relative, through a link in it", gitdir: () => `m/${G}`, answered: true, reads: 1 },
  { name: "absolute, with a . name", gitdir: root => `${join(root, "repos")}/./main/${G}`, answered: false, reads: 0 },
  { name: "absolute, with link/..", gitdir: root => `${join(root, "hop")}/../main/${G}`, answered: false, flat: true, reads: 0 },
  // A rooted path with no drive letter on Windows, which Git reads from the drive it changed into.
  { name: "rooted, with a ..", gitdir: root => `${WINDOWS ? root.slice(2) : root}/wts/../repos/main/${G}`, answered: false, reads: 0 },
  { name: "relative, with a . name", gitdir: () => `../../repos/main/${G}/.`, answered: false, reads: 0 },
  { name: "relative, with link/..", gitdir: () => `../../hop/../main/${G}`, answered: false, flat: true, reads: 0 },
  { name: "relative, with a .. after a link whose folder is gone", gitdir: () => `../../dead/../repos/main/${G}`, answered: false, reads: 0 },
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

/** A row makes a repository and a worktree with Git, records it and asks Git again: the first rows load the modules too. */
const ROW_TIMEOUT = 30_000;

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
  readonly worktree: string;
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
  mkdirSync(join(root, "gone"));
  linkFolder(join(root, "gone"), join(root, "dead"));
  removeFolderSync(join(root, "gone"));
  linkFolder(main, join(worktree, "m"));
  const gitdir = row.spelling.gitdir(root);
  writeGitFile(worktree, gitdir);
  mkdirSync(join(root, "links", "x"), { recursive: true });
  const asked = row.reach === "own path" ? worktree : join(root, "links", "x", "current");
  if (row.reach !== "own path") linkAs(context, row.reach, worktree, asked);
  return { worktree, asked, gitdir, reaches: join(root, ...(WINDOWS && row.spelling.flat === true ? [] : ["repos"]), "main", G) };
}

/** Writes `worktree`'s `.git` file to name its Git folder by `gitdir`. */
function writeGitFile(worktree: string, gitdir: string): void {
  // Written anew: Git for Windows marks the .git file hidden, and Windows refuses to open a hidden file to replace it.
  unlinkSync(join(worktree, ".git"));
  writeFileSync(join(worktree, ".git"), `gitdir: ${gitdir}\n`);
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

/** How many Git processes a placement of `folder` starts, once it is placed as Git places it (`placed`). */
function askedAgain(folder: string, placed: boolean): number {
  const before = locateCalls(folder);
  expect(repositoryIdentityLocation(folder).git).toBe(placed);
  return locateCalls(folder) - before;
}

async function temporaryRoot(): Promise<string> {
  const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-git-folder-reached-")));
  roots.push(root);
  return root;
}

/** A row made, recorded and its main checkout handled, with Git's answer and Rerail's, read after. */
async function rowRun(row: Row, context: TestContext) {
  const root = await temporaryRoot();
  const rowMade = made(context, root, row);
  const db = openRerailDatabase(join(root, "rerail.sqlite"));
  databases.push(db);
  const recorded = recordCheckout(db, rowMade.asked);
  afterRecording(root, row.after);
  const answer = gitsAnswer(rowMade.asked);
  const location = repositoryIdentityLocation(rowMade.asked);
  return { ...rowMade, db, recorded, answer, placed: answer.status === 0, location };
}

describe("a .git file's path in a form Git writes: Rerail answers, equal to Git", () => {
  it.for(ROWS.filter(row => row.spelling.answered))("$name", { timeout: ROW_TIMEOUT }, async (row, context) => {
    const { asked, gitdir, reaches, db, recorded, answer, placed, location } = await rowRun(row, context);
    const gone = !placed && missing(reaches);
    expect(placed ? storedForm(answer.gitDir!) : answer.status, "Git places the worktree where its Git folder is there").toBe(placed ? storedForm(reaches) : 128);

    // Every reader: the placement, the gone check and its `worktrees` name, and the facts an acceptance is kept under.
    const read = { git: location.git, workspaceRoot: location.workspaceRoot, gitFolderGone: location.gitFolderGone };
    expect(read).toEqual({ git: placed, workspaceRoot: placed ? answer.top : resolve(asked), gitFolderGone: gone ? true : undefined });
    if (placed) expect(gitEntryFacts(asked), "the facts hold the Git folder's real path").toContain(`\n${checkoutRealpath(answer.gitDir!)}\n`);
    if (placed) expect(askedAgain(asked, true), "Git processes a second placement starts").toBe(0);
    if (gone) expect(recordCheckout(db, asked).cloneId, "the recorded clone id is kept").toBe(recorded.cloneId);

    // The function against Git's answer, and the real paths it read to reach it.
    const before = realPathReads();
    const reached = gitFolderReached(asked, gitdir);
    expect(realPathReads() - before, "real paths read").toBe(row.spelling.reads);
    if (gone) expect(reached !== undefined && missing(reached), "the folder it reaches is gone").toBe(true);
    else expect(reached === undefined ? undefined : storedForm(reached)).toBe(storedForm(answer.gitDir!));
  });
});

describe("a .git file's path in any other form: handed to Git, the answer is Git's", () => {
  it.for(ROWS.filter(row => !row.spelling.answered))("$name", { timeout: ROW_TIMEOUT }, async (row, context) => {
    const { asked, gitdir, answer, placed, location } = await rowRun(row, context);

    const read = { git: location.git, workspaceRoot: location.workspaceRoot, gitFolderGone: location.gitFolderGone };
    expect(read).toEqual({ git: placed, workspaceRoot: placed ? answer.top : resolve(asked), gitFolderGone: undefined });
    expect(askedAgain(asked, placed), "Git processes a second placement starts").toBe(1);
    expect(gitEntryFacts(asked), "no facts kept for Git's answer").toBeUndefined();
    const before = realPathReads();
    expect(gitFolderReached(asked, gitdir)).toBeUndefined();
    expect(realPathReads() - before, "real paths read").toBe(0);
  });

  // A drive letter `subst` maps to the case's root: Git reads a rooted path there from that drive, not the process's.
  it.runIf(WINDOWS).for(["there", "deleted"] as const)("rooted, with a .., asked on another drive than the process's working folder, main checkout %s", async (after, context) => {
    const root = await temporaryRoot();
    const drive = substitutedDrive(context, root);
    const { worktree } = made(context, root, { spelling: SPELLINGS.find(spelling => spelling.name === "rooted, with a ..")!, reach: "own path", after });
    const gitdir = `/wts/../repos/main/${G}`;
    writeGitFile(worktree, gitdir);
    afterRecording(root, after);
    const asked = `${drive}\\wts\\feature`;
    expect(process.cwd().slice(0, 2).toUpperCase(), "the process works on another drive").not.toBe(drive);
    const answer = gitsAnswer(asked);
    expect(answer.status, "Git reaches the main checkout's Git folder on the asked folder's drive").toBe(after === "there" ? 0 : 128);

    const location = repositoryIdentityLocation(asked);
    expect({ git: location.git, workspaceRoot: location.workspaceRoot, gitFolderGone: location.gitFolderGone })
      .toEqual({ git: after === "there", workspaceRoot: after === "there" ? answer.top : resolve(asked), gitFolderGone: undefined });
    expect(gitFolderReached(asked, gitdir)).toBeUndefined();
  });
});

/** A free drive letter mapped to `root` by `subst`, or the case skipped, naming why, where none can be mapped. */
function substitutedDrive(context: TestContext, root: string): string {
  const drive = [..."RSTUVWXYZ"].map(letter => `${letter}:`).find(letter => !existsSync(`${letter}\\`));
  if (drive === undefined) context.skip("no drive letter is free for subst");
  const mapped = spawnSync("subst", [drive!, root], { stdio: "ignore" });
  if (mapped.status !== 0) context.skip(`subst ${drive} refused, exit ${mapped.status}`);
  substituted.push(drive!);
  return drive!;
}

describe("the two forms Git writes into a .git file, read as each system reads a path", () => {
  it.for([
    { written: "/repos/main/.git/worktrees/feature", platform: "linux", form: "absolute" },
    { written: "../../repos/main/.git/worktrees/feature", platform: "linux", form: "relative" },
    { written: "m/.git/worktrees/feature", platform: "linux", form: "relative" },
    { written: "/repos/./main/.git/worktrees/feature", platform: "linux", form: undefined },
    { written: "/wts/../repos/main/.git/worktrees/feature", platform: "linux", form: undefined },
    { written: "/repos//main/.git/worktrees/feature", platform: "linux", form: undefined },
    { written: "/repos/main/.git/worktrees/feature/", platform: "linux", form: undefined },
    { written: "../../hop/../main/.git/worktrees/feature", platform: "linux", form: undefined },
    { written: "../..", platform: "linux", form: undefined },
    { written: "D:\\repos\\main\\.git\\worktrees\\feature", platform: "win32", form: "absolute" },
    { written: "D:/repos/main/.git/worktrees/feature", platform: "win32", form: "absolute" },
    { written: "..\\..\\repos\\main\\.git\\worktrees\\feature", platform: "win32", form: "relative" },
    // A rooted path with no drive letter: Git reads it from the drive it changed into, Node's resolve from the process's.
    { written: "/wts/../repos/main/.git/worktrees/feature", platform: "win32", form: undefined },
    { written: "\\repos\\main\\.git\\worktrees\\feature", platform: "win32", form: undefined },
    { written: "D:\\repos\\main.\\.git\\worktrees\\feature", platform: "win32", form: undefined },
    { written: "D:\\repos\\main \\.git\\worktrees\\feature", platform: "win32", form: undefined },
    { written: "D:\\repos\\.\\main\\.git\\worktrees\\feature", platform: "win32", form: undefined },
    { written: "D:repos\\main\\.git\\worktrees\\feature", platform: "win32", form: undefined },
    { written: "D:", platform: "win32", form: undefined },
  ] as const)("$platform: $written", ({ written, platform, form }) => {
    expect(writtenForm(written, platform)).toBe(form);
  });
});
