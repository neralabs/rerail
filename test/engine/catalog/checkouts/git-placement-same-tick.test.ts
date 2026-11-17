// A file Git reads to place a folder, rewritten within one tick of the file system's clock at the same size, keeps the
// lstat it had: only its text tells the change, at any later use. Node's lstat is wrapped here so a test can hand back
// the lstat a file had before such a rewrite, which a fast enough rewrite gives on a machine with a coarse clock (Linux
// 6.12 on its second try; NTFS in most of 300 tries), or the lstat of an owner changed with chown.
import { execFileSync, spawnSync } from "node:child_process";
import { lstatSync, mkdirSync, realpathSync, type Stats } from "node:fs";
import { mkdtemp, open, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { repositoryIdentityLocation, type RepositoryIdentityLocation } from "../../../../src/engine/catalog/repository-identity.js";
import { runGit } from "../../../../src/engine/git.js";
import { removeFolder } from "../../../support/platform/remove-folder.js";

vi.mock("../../../../src/engine/git.js", async importOriginal => {
  const actual = await importOriginal<typeof import("../../../../src/engine/git.js")>();
  return { ...actual, runGit: vi.fn(actual.runGit) };
});

vi.mock("node:fs", async importOriginal => {
  const fs = await importOriginal<typeof import("node:fs")>();
  return { ...fs, lstatSync: vi.fn(fs.lstatSync) };
});

const realLstat = (await vi.importActual<typeof import("node:fs")>("node:fs")).lstatSync;

const cleanups: Array<() => Promise<void>> = [];
afterEach(async () => {
  vi.mocked(lstatSync).mockReset();
  vi.mocked(lstatSync).mockImplementation(realLstat);
  vi.useRealTimers();
  for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

function repository(path: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "--quiet");
  git(path, "commit", "--quiet", "--allow-empty", "-m", "seed");
  return realpathSync.native(path);
}

function gitsAnswer(folder: string): RepositoryIdentityLocation {
  const requested = resolve(folder);
  const result = spawnSync("git", ["-C", requested, "rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"], { encoding: "utf8" });
  if (result.status !== 0) return { git: false, workspaceRoot: requested, identityPath: join(requested, ".rerail", "repository-id"), main: true };
  const [top, common, gitDirectory] = result.stdout.trim().split("\n").map(line => resolve(line.trim()));
  return { git: true, workspaceRoot: top!, identityPath: join(common!, "rerail-repository-id"), main: common === gitDirectory };
}

/** How many Git processes Rerail started to place `folder`. */
function placements(folder: string): number {
  return vi.mocked(runGit).mock.calls.filter(([cwd, args]) => cwd === resolve(folder) && args.includes("--show-toplevel")).length;
}

/**
 * Writes `text` over the file at `path` in place, as Git rewrites a `.git` file. Git for Windows hides a worktree's
 * `.git` file, and Windows refuses to make a file anew over a hidden one.
 */
async function rewriteInPlace(path: string, text: string): Promise<void> {
  const file = await open(path, "r+");
  try {
    await file.truncate(0);
    await file.write(text, 0);
  } finally {
    await file.close();
  }
}

/** From now on, an lstat of `path` gives `stats`, as if nothing had changed there since. */
function lstatKeeps(path: string, stats: Stats): void {
  vi.mocked(lstatSync).mockImplementation(((target: Parameters<typeof lstatSync>[0], options?: Parameters<typeof lstatSync>[1]) =>
    resolve(String(target)) === path ? stats : realLstat(target, options)) as typeof lstatSync);
}

describe("a file Git reads to place a folder, rewritten within one clock tick at the same size", () => {
  it("a worktree's .git file rewritten to name another repository's worktree is placed as Git places it", async () => {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-same-tick-")));
    cleanups.push(() => removeFolder(root));
    // Two repositories whose worktree Git folders are spelled at the same length, so the rewrite keeps the size.
    const first = repository(join(root, "aa"));
    const other = repository(join(root, "bb"));
    git(first, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    git(other, "worktree", "add", "--quiet", "-b", "w", join(root, "o", "w"));
    const dotGit = join(root, "w", ".git");
    const before = realLstat(dotGit);
    // Every read below happens within the clock tick of the .git file's last write.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(Math.max(before.mtimeMs, before.ctimeMs));

    expect(repositoryIdentityLocation(first)).toEqual(gitsAnswer(first));
    expect(repositoryIdentityLocation(join(root, "w"))).toEqual(gitsAnswer(join(root, "w")));
    await rewriteInPlace(dotGit, `gitdir: ${join(other, ".git", "worktrees", "w")}\n`);
    lstatKeeps(dotGit, before);

    expect(repositoryIdentityLocation(join(root, "w"))).toEqual(gitsAnswer(join(root, "w")));
    expect(gitsAnswer(join(root, "w")).identityPath).toBe(join(other, ".git", "rerail-repository-id"));
  });

  it("the same rewrite, asked again a minute after the tick, is placed as Git places it", async () => {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-same-tick-")));
    cleanups.push(() => removeFolder(root));
    const first = repository(join(root, "aa"));
    const other = repository(join(root, "bb"));
    git(first, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    git(other, "worktree", "add", "--quiet", "-b", "w", join(root, "o", "w"));
    const dotGit = join(root, "w", ".git");
    const before = realLstat(dotGit);
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(Math.max(before.mtimeMs, before.ctimeMs));
    expect(repositoryIdentityLocation(first)).toEqual(gitsAnswer(first));
    expect(repositoryIdentityLocation(join(root, "w"))).toEqual(gitsAnswer(join(root, "w")));
    await rewriteInPlace(dotGit, `gitdir: ${join(other, ".git", "worktrees", "w")}\n`);
    lstatKeeps(dotGit, before);

    vi.setSystemTime(Math.max(before.mtimeMs, before.ctimeMs) + 60_000);
    expect(repositoryIdentityLocation(join(root, "w"))).toEqual(gitsAnswer(join(root, "w")));
    expect(gitsAnswer(join(root, "w")).identityPath).toBe(join(other, ".git", "rerail-repository-id"));
  });

  it("a repository's config rewritten at the same size to make it bare is placed as Git places it, a minute after the tick", async () => {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-same-tick-")));
    cleanups.push(() => removeFolder(root));
    const main = repository(join(root, "aa"));
    const config = join(main, ".git", "config");
    const text = await readFile(config, "utf8");
    const bare = text.replace(/bare = false/u, "bare =  true");
    expect(bare).not.toBe(text);
    const before = realLstat(config);
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(Math.max(before.mtimeMs, before.ctimeMs));
    expect(repositoryIdentityLocation(main)).toEqual(gitsAnswer(main));
    await rewriteInPlace(config, bare);
    lstatKeeps(config, before);

    vi.setSystemTime(Math.max(before.mtimeMs, before.ctimeMs) + 60_000);
    expect(repositoryIdentityLocation(main)).toEqual(gitsAnswer(main));
    expect(gitsAnswer(main).git).toBe(false);
  });

  it("keeps the answer, with no Git process, for a file whose text is the same, and for a file older than the clock's tick", async () => {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-same-tick-")));
    cleanups.push(() => removeFolder(root));
    const first = repository(join(root, "aa"));
    git(first, "worktree", "add", "--quiet", "-b", "w", join(root, "w"));
    const dotGit = join(root, "w", ".git");
    const before = realLstat(dotGit);
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(Math.max(before.mtimeMs, before.ctimeMs));
    const answer = repositoryIdentityLocation(join(root, "w"));
    const placed = placements(join(root, "w"));

    lstatKeeps(dotGit, before);
    expect(repositoryIdentityLocation(join(root, "w"))).toEqual(answer);
    vi.setSystemTime(Math.max(before.mtimeMs, before.ctimeMs) + 60_000);
    expect(repositoryIdentityLocation(join(root, "w"))).toEqual(answer);
    expect({ answer, gitAfterTheFirst: placements(join(root, "w")) - placed }).toEqual({ answer: gitsAnswer(join(root, "w")), gitAfterTheFirst: 0 });
  });
});

describe.skipIf(process.platform === "win32")("a kept answer on POSIX, where an lstat gives each folder's owner", () => {
  it("is placed again once the checkout's folder is another account's, as Git reads its owner at every start", async () => {
    const root = realpathSync.native(await mkdtemp(join(tmpdir(), "rerail-same-tick-")));
    cleanups.push(() => removeFolder(root));
    const main = repository(join(root, "aa"));
    const answer = repositoryIdentityLocation(main);
    const placed = placements(main);
    const stats = realLstat(main);

    lstatKeeps(main, Object.assign(Object.create(Object.getPrototypeOf(stats) as object) as Stats, stats, { uid: stats.uid + 1 }));
    expect(repositoryIdentityLocation(main)).toEqual(answer);
    expect(repositoryIdentityLocation(main)).toEqual(answer);
    expect({ answer, gitAfterTheFirst: placements(main) - placed }).toEqual({ answer: gitsAnswer(main), gitAfterTheFirst: 1 });
  });
});
