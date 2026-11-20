import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/** The arguments of each Git process Rerail started; the test's own `git` uses `execFileSync`, which is not counted. */
const started = vi.hoisted(() => ({ git: [] as string[][] }));
vi.mock("node:child_process", async importOriginal => {
  const original = await importOriginal<typeof import("node:child_process")>();
  const spawn = ((file: string, args: readonly string[], ...rest: never[]) => {
    if (/(^|[\\/])git(\.exe)?$/iu.test(file)) started.git.push([...args]);
    return original.spawn(file, args, ...rest);
  }) as typeof original.spawn;
  return { ...original, spawn };
});

import { gitHead } from "../../src/engine/git.js";
import { gitHeadReader, type GitHeadReader } from "../../src/engine/git-head-reader.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const folders: string[] = [];
const environment = new Map<string, string | undefined>();
afterEach(async () => {
  for (const [name, value] of environment) {
    if (value === undefined) delete process.env[name];
    else process.env[name] = value;
  }
  environment.clear();
  await Promise.all(folders.splice(0).map(async folder => await removeFolder(folder)));
});

/** Sets an environment variable for this test only. */
function setVariable(name: string, value: string): void {
  if (!environment.has(name)) environment.set(name, process.env[name]);
  process.env[name] = value;
}

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
}

/** A repository at `main` on branch `main` with one commit, and a linked worktree `w` on branch `feature`. */
async function repository(): Promise<{ root: string; main: string; worktree: string }> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-head-reader-")));
  folders.push(root);
  const main = join(root, "main");
  await mkdir(join(main, "src"), { recursive: true });
  await writeFile(join(main, "src", "a.py"), "a = 1\n");
  git(main, "init", "--quiet", "--initial-branch=main");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  const worktree = join(root, "w");
  git(main, "worktree", "add", "--quiet", "-b", "feature", worktree);
  return { root, main, worktree };
}

/**
 * The reader's answer for `cwd`, the answer `gitHead` gives right after, the Git processes the reader started, and how
 * many of them asked for `HEAD` (`rev-parse`).
 */
async function read(reader: GitHeadReader, cwd: string): Promise<{ head: string | undefined; git: number; heads: number; expected: string | undefined }> {
  const before = started.git.length;
  const head = await reader(cwd);
  const asked = started.git.slice(before);
  return { head, git: asked.length, heads: asked.filter(args => args.includes("rev-parse")).length, expected: await gitHead(cwd) };
}

/** Commits a change to `src/a.py` in `cwd`. */
async function commit(cwd: string, text: string): Promise<void> {
  await writeFile(join(cwd, "src", "a.py"), text);
  git(cwd, "commit", "--quiet", "-am", text);
}

/** A reader whose owners never change and whose inputs count as settled at once, so only the files decide. */
const filesReader = (): GitHeadReader => gitHeadReader({ settleMs: 0, owners: async () => "owner" });

describe("gitHeadReader", () => {
  it("names HEAD as gitHead does through commits, branch switches, packed refs, a detached HEAD and an unborn branch, asking Git only at a checkout's first read", async () => {
    const { main, worktree } = await repository();
    const reader = filesReader();
    const steps: Array<[string, () => Promise<void> | void]> = [
      ["the first read", () => undefined],
      ["a read again", () => undefined],
      ["a commit", async () => await commit(main, "a = 2\n")],
      ["a commit in the worktree", async () => await commit(worktree, "a = 3\n")],
      ["a new branch", () => void git(main, "switch", "--quiet", "-c", "other")],
      ["every ref packed", () => void git(main, "pack-refs", "--all", "--prune")],
      ["a commit after the packing", async () => await commit(main, "a = 4\n")],
      ["a switch to a packed branch", () => void git(main, "switch", "--quiet", "main")],
      ["a detached HEAD in the worktree", () => void git(worktree, "switch", "--quiet", "--detach")],
      ["an unborn branch", () => void git(main, "switch", "--quiet", "--orphan", "fresh")],
      ["the branch back", () => void git(main, "switch", "--quiet", "main")],
    ];
    const seen: string[] = [];
    for (const [step, change] of steps) {
      await change();
      for (const [name, cwd] of [["main", main], ["worktree", worktree]] as const) {
        const answer = await read(reader, cwd);
        expect(answer.head, `${step}, ${name}`).toBe(answer.expected);
        seen.push(`${step}, ${name}: ${answer.git}`);
      }
    }
    // Git is asked once per checkout, at its first read, beside the reader's one question of the global and system files'
    // names (`git var`, twice); every later answer comes from the files.
    expect(seen.filter(line => !line.endsWith(": 0"))).toEqual(["the first read, main: 3", "the first read, worktree: 1"]);
  });

  it("asks Git once a configuration file it reads changed, and names no commit while Git cannot parse one", async () => {
    const { root, main, worktree } = await repository();
    const global = join(root, "global.cfg");
    const included = join(root, "included.cfg");
    await writeFile(global, "[user]\n\tname = Rerail\n");
    await writeFile(included, "[core]\n\tabbrev = 12\n");
    setVariable("GIT_CONFIG_GLOBAL", global);
    git(main, "config", "include.path", included);
    git(main, "config", "core.repositoryFormatVersion", "1");
    git(main, "config", "extensions.worktreeConfig", "true");
    await writeFile(join(main, ".git", "worktrees", "w", "config.worktree"), "[core]\n\tabbrev = 10\n");
    const reader = filesReader();
    const files: Array<[string, string]> = [
      ["the repository's config", join(main, ".git", "config")],
      ["the worktree's config.worktree", join(main, ".git", "worktrees", "w", "config.worktree")],
      ["the global file", global],
      ["an included file", included],
    ];
    for (const [name, path] of files) {
      const good = await readFile(path, "utf8");
      for (const cwd of [main, worktree]) expect((await read(reader, cwd)).head).toBeDefined();
      for (const cwd of [main, worktree]) expect((await read(reader, cwd)).git, `${name}: a read before the change`).toBe(0);
      await writeFile(path, `${good}[broken\n`);
      for (const cwd of [main, worktree]) {
        const broken = await read(reader, cwd);
        if (name !== "the worktree's config.worktree" || cwd === worktree) {
          expect(broken.expected, `${name}: Git refuses the checkout`).toBeUndefined();
          expect(broken.git, `${name}: Git asked`).toBeGreaterThan(0);
        }
        expect(broken.head, name).toBe(broken.expected);
      }
      await writeFile(path, good);
      for (const cwd of [main, worktree]) {
        const fixed = await read(reader, cwd);
        expect(fixed.head, `${name}, fixed`).toBe(fixed.expected);
        expect(fixed.head, `${name}, fixed`).toBeDefined();
      }
    }
  });

  it("asks Git again once an owner Git checks changed", async () => {
    const { main } = await repository();
    let owner = "one account";
    const reader = gitHeadReader({ settleMs: 0, owners: async () => owner });
    expect((await read(reader, main)).heads).toBe(1);
    expect((await read(reader, main)).git).toBe(0);
    owner = "another account";
    const after = await read(reader, main);
    expect(after.heads).toBe(1);
    expect(after.head).toBe(after.expected);
    expect((await read(reader, main)).git).toBe(0);
  });

  it("reads owners as Git checks them: the user id on POSIX, where a write in the Git folder asks nothing; on Windows each folder's stamp, where it asks Git again", async () => {
    const { main } = await repository();
    const reader = gitHeadReader({ settleMs: 0 });
    expect((await read(reader, main)).heads).toBe(1);
    expect((await read(reader, main)).git).toBe(0);
    await writeFile(join(main, ".git", "rerail-probe"), "");
    const after = await read(reader, main);
    expect(after.head).toBe(after.expected);
    expect(after.heads).toBe(process.platform === "win32" ? 1 : 0);
  });

  it("asks Git on every read where Git's files do not decide alone", async () => {
    const { main } = await repository();
    const cases: Array<[string, () => Promise<string> | string]> = [
      ["a branch that is a symbolic ref to another", () => {
        git(main, "symbolic-ref", "refs/heads/alias", "refs/heads/main");
        git(main, "symbolic-ref", "HEAD", "refs/heads/alias");
        return main;
      }],
      ["a ref file that holds no object id", async () => {
        git(main, "switch", "--quiet", "-c", "broken");
        await writeFile(join(main, ".git", "refs", "heads", "broken"), "not an object id\n");
        return main;
      }],
      ["a folder below the checkout's top", () => join(main, "src")],
      ["a branch name with a part that ends with a dot, which Windows reads as another file", async () => {
        // A file holds a commit at the branch's path, as Windows finds `dots/x` there, while Git refuses the name.
        await mkdir(join(main, ".git", "refs", "heads", "dots."), { recursive: true });
        await writeFile(join(main, ".git", "refs", "heads", "dots.", "x"), `${git(main, "rev-parse", "main")}\n`);
        git(main, "symbolic-ref", "HEAD", "refs/heads/dots./x");
        return main;
      }],
    ];
    for (const [name, arrange] of cases) {
      const reader = filesReader();
      // Git's answer for the checkout as it was vouches for its files, which each case then changes.
      expect((await read(reader, main)).heads).toBe(1);
      const cwd = await arrange();
      for (const time of [1, 2, 3]) {
        const answer = await read(reader, cwd);
        expect(answer.head, `${name}, read ${time}`).toBe(answer.expected);
        expect(answer.heads, `${name}, read ${time}`).toBe(1);
      }
      git(main, "symbolic-ref", "HEAD", "refs/heads/main");
    }
  });

  it("asks Git on every read while GIT_DIR moves where Git looks", async () => {
    const { main, worktree } = await repository();
    setVariable("GIT_DIR", join(main, ".git"));
    const reader = filesReader();
    for (const time of [1, 2]) {
      const answer = await read(reader, worktree);
      expect(answer.head, `read ${time}`).toBe(answer.expected);
      expect(answer.head).toBe(git(main, "rev-parse", "main"));
      expect(answer.heads, `read ${time}`).toBe(1);
    }
  });

  it("starts one Git process a read in a folder outside any repository", async () => {
    const outside = await realpath(await mkdtemp(join(tmpdir(), "rerail-head-reader-")));
    folders.push(outside);
    const reader = filesReader();
    for (const time of [1, 2]) {
      const answer = await read(reader, outside);
      expect(answer.expected, `read ${time}`).toBeUndefined();
      expect(answer.head, `read ${time}`).toBeUndefined();
      expect(answer.git, `read ${time}`).toBe(1);
    }
  });

  it("asks Git on every read while a configuration file was written less than the settle time ago", async () => {
    const { main } = await repository();
    const reader = gitHeadReader({ settleMs: 600_000, owners: async () => "owner" });
    for (const time of [1, 2, 3]) {
      const answer = await read(reader, main);
      expect(answer.head).toBe(answer.expected);
      expect(answer.heads, `read ${time}`).toBe(1);
    }
  });
});
