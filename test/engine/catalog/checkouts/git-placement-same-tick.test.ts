// A file Git reads to place a folder, rewritten within one tick of the file system's clock at the same size, keeps the
// lstat it had: only its text tells the change. Node's lstat is wrapped here so a test can hand back the lstat a file
// had before such a rewrite, which a fast enough rewrite gives on a machine with a coarse clock.
import { execFileSync, spawnSync } from "node:child_process";
import { lstatSync, mkdirSync, realpathSync, type Stats } from "node:fs";
import { mkdtemp, writeFile } from "node:fs/promises";
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
    await writeFile(dotGit, `gitdir: ${join(other, ".git", "worktrees", "w")}\n`);
    lstatKeeps(dotGit, before);

    expect(repositoryIdentityLocation(join(root, "w"))).toEqual(gitsAnswer(join(root, "w")));
    expect(gitsAnswer(join(root, "w")).identityPath).toBe(join(other, ".git", "rerail-repository-id"));
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
