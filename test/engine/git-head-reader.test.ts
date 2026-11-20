import { execFileSync } from "node:child_process";
import type { PathLike } from "node:fs";
import { cp, mkdir, mkdtemp, readFile, realpath, symlink, unlink, writeFile } from "node:fs/promises";
import { tmpdir, userInfo } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

/**
 * The arguments of each Git process Rerail started; the test's own `git` uses `execFileSync`, which is not counted. A
 * `git var` of a variable in `failing` fails once, as one in a folder just removed does; with `oldGit`, every `git var`
 * asks a variable Git does not know, as before Git 2.42.
 */
const started = vi.hoisted(() => ({ git: [] as string[][], failing: new Set<string>(), oldGit: false }));
vi.mock("node:child_process", async importOriginal => {
  const original = await importOriginal<typeof import("node:child_process")>();
  const spawn = ((file: string, args: readonly string[], ...rest: never[]) => {
    if (!/(^|[\\/])git(\.exe)?$/iu.test(file)) return original.spawn(file, args, ...rest);
    started.git.push([...args]);
    const variable = args.at(-1) ?? "";
    if (args.at(-2) !== "var") return original.spawn(file, args, ...rest);
    if (started.failing.delete(variable)) return original.spawn(file, [...args.slice(0, -2), "-C", join(tmpdir(), "rerail-no-such-folder"), "var", variable], ...rest);
    return original.spawn(file, started.oldGit ? [...args.slice(0, -1), "GIT_RERAIL_NO_SUCH_VARIABLE"] : args, ...rest);
  }) as typeof original.spawn;
  return { ...original, spawn };
});

/**
 * Files whose `stat` fails as an unreadable file's does (EACCES), and files whose stamp holds still through writes, as
 * on a file system whose clock ticks coarsely: the same inode, size and times, `timeMs` ago.
 */
const statFaults = vi.hoisted(() => ({ refused: new Set<string>(), frozen: new Map<string, number>() }));
vi.mock("node:fs/promises", async importOriginal => {
  const original = await importOriginal<typeof import("node:fs/promises")>();
  const stat = (async (path: PathLike, ...rest: never[]) => {
    if (statFaults.refused.has(String(path))) throw Object.assign(new Error(`EACCES: permission denied, stat '${String(path)}'`), { code: "EACCES" });
    const info = await original.stat(path, ...rest);
    const timeMs = statFaults.frozen.get(String(path));
    return timeMs === undefined ? info : Object.assign(info, { ino: 1, size: 1, mtimeMs: timeMs, ctimeMs: timeMs });
  }) as typeof original.stat;
  return { ...original, stat };
});

import { gitHead } from "../../src/engine/git.js";
import { gitHeadReader, type GitHeadReader } from "../../src/engine/git-head-reader.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const folders: string[] = [];
const environment = new Map<string, string | undefined>();
afterEach(async () => {
  started.failing.clear();
  started.oldGit = false;
  statFaults.refused.clear();
  statFaults.frozen.clear();
  vi.restoreAllMocks();
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

/** `path` with `/` separators, as a configuration file can hold it on any system (a `\` there starts an escape). */
function slash(path: string): string {
  return path.replaceAll("\\", "/");
}

/** A `[rerail] probe = <name>` file at `path`, which `git config --get rerail.probe` names wherever Git reads it. */
async function probeFile(path: string, name: string, extra = ""): Promise<void> {
  await writeFile(path, `[rerail]\n\tprobe = ${name}\n${extra}`);
}

/** Writes `path` so that Git refuses every checkout that reads it, then reads `cwd` once: Git and the reader both. */
async function broken(reader: GitHeadReader, cwd: string, path: string): Promise<Awaited<ReturnType<typeof read>>> {
  await writeFile(path, `${await readFile(path, "utf8")}[broken\n`);
  return await read(reader, cwd);
}

/** A clock `aheadMs` ahead of the real one, for the reader's settle time: a test moves it on in place of waiting. */
function aheadClock(): { aheadMs: number } {
  const clock = { aheadMs: 0 };
  const realNow = Date.now.bind(Date);
  vi.spyOn(Date, "now").mockImplementation(() => realNow() + clock.aheadMs);
  return clock;
}

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

  it("asks Git on every read while a configuration file was written less than the settle time ago", async () => {
    const { main } = await repository();
    const reader = gitHeadReader({ settleMs: 600_000, owners: async () => "owner" });
    for (const time of [1, 2, 3]) {
      const answer = await read(reader, main);
      expect(answer.head).toBe(answer.expected);
      expect(answer.heads, `read ${time}`).toBe(1);
    }
  });

  it("asks Git while packed-refs cannot be stamped, never answering from the branches it read there before", async () => {
    const { main } = await repository();
    git(main, "pack-refs", "--all", "--prune");
    const reader = filesReader();
    expect((await read(reader, main)).heads).toBe(1);
    const fromFiles = await read(reader, main);
    expect(fromFiles.git).toBe(0);
    expect(fromFiles.head).toBe(fromFiles.expected);
    // A commit, then every ref packed again: the branch's commit is in packed-refs alone, which no other input follows.
    await commit(main, "a = 2\n");
    git(main, "pack-refs", "--all", "--prune");
    statFaults.refused.add(join(main, ".git", "packed-refs"));
    for (const time of [1, 2]) {
      const answer = await read(reader, main);
      expect(answer.head, `read ${time}`).toBe(answer.expected);
      expect(answer.heads, `read ${time}`).toBe(1);
    }
  });

  const includes: Array<[string, (main: string, root: string) => Promise<string>]> = [
    ["a relative path, in the folder of the repository's config", async main => {
      git(main, "config", "include.path", "relative.cfg");
      return join(main, ".git", "relative.cfg");
    }],
    ["a relative path in an included file, in that file's folder", async main => {
      await mkdir(join(main, ".git", "nested"));
      await writeFile(join(main, ".git", "nested", "outer.cfg"), "[include]\n\tpath = inner.cfg\n");
      git(main, "config", "include.path", "nested/outer.cfg");
      return join(main, ".git", "nested", "inner.cfg");
    }],
    ["a path in Git's home (`~/`)", async (_, root) => {
      await mkdir(join(root, "home"));
      setVariable("HOME", join(root, "home"));
      git(join(root, "main"), "config", "include.path", "~/home.cfg");
      return join(root, "home", "home.cfg");
    }],
    ["an includeIf whose gitdir condition holds, with a relative path", async main => {
      git(main, "config", `includeIf.gitdir/i:${slash(main)}/.path`, "conditional.cfg");
      return join(main, ".git", "conditional.cfg");
    }],
  ];
  it.each(includes)("asks Git once a file an include names changed: %s", async (name, arrange) => {
    const { root, main } = await repository();
    const target = await arrange(main, root);
    await probeFile(target, name);
    // Git's own answer: it reads the file the include names.
    expect(git(main, "config", "--get", "rerail.probe")).toBe(name);
    const reader = filesReader();
    expect((await read(reader, main)).head).toBeDefined();
    expect((await read(reader, main)).git).toBe(0);
    const refused = await broken(reader, main, target);
    expect(refused.expected).toBeUndefined();
    expect(refused.head).toBe(refused.expected);
  });

  it("asks Git once an include added to an included file later names a file that changed", async () => {
    const { root, main, worktree } = await repository();
    const [outer, inner] = [join(root, "outer.cfg"), join(root, "inner.cfg")];
    await probeFile(outer, "outer");
    await probeFile(inner, "inner");
    git(main, "config", "include.path", slash(outer));
    const reader = filesReader();
    for (const cwd of [main, worktree]) expect((await read(reader, cwd)).head).toBeDefined();
    await probeFile(outer, "outer", `[include]\n\tpath = ${slash(inner)}\n`);
    expect(git(main, "config", "--get", "rerail.probe")).toBe("inner");
    for (const cwd of [main, worktree]) {
      const answer = await read(reader, cwd);
      expect(answer.head).toBe(answer.expected);
    }
    for (const cwd of [main, worktree]) expect((await read(reader, cwd)).git).toBe(0);
    await probeFile(inner, "inner", "[broken\n");
    for (const cwd of [main, worktree]) {
      const refused = await read(reader, cwd);
      expect(refused.expected).toBeUndefined();
      expect(refused.head).toBe(refused.expected);
    }
  });

  it("follows an onbranch include both ways: unread off its branch, and once on it the files it includes in turn", async () => {
    const { root, main } = await repository();
    const [onOther, nested] = [join(root, "on-other.cfg"), join(root, "nested.cfg")];
    await probeFile(nested, "nested");
    await writeFile(onOther, `[include]\n\tpath = ${slash(nested)}\n`);
    git(main, "config", "includeIf.onbranch:other.path", slash(onOther));
    git(main, "branch", "other");
    const reader = filesReader();
    expect((await read(reader, main)).head).toBeDefined();
    // On `main` Git reads neither file: a change to them leaves its answer.
    expect(git(main, "config", "--default", "none", "--get", "rerail.probe")).toBe("none");
    const unread = await broken(reader, main, nested);
    expect(unread.expected).toBeDefined();
    expect(unread.head).toBe(unread.expected);
    await probeFile(nested, "nested");
    git(main, "switch", "--quiet", "other");
    expect(git(main, "config", "--get", "rerail.probe")).toBe("nested");
    const switched = await read(reader, main);
    expect(switched.head).toBe(switched.expected);
    expect((await read(reader, main)).git).toBe(0);
    const refused = await broken(reader, main, nested);
    expect(refused.expected).toBeUndefined();
    expect(refused.head).toBe(refused.expected);
  });

  it("reads a configuration file for an include section again until its stamp has settled", async () => {
    const { root, main } = await repository();
    const included = join(root, "included.cfg");
    await probeFile(included, "included");
    const clock = aheadClock();
    // The repository's config keeps one stamp through the write below, as in one tick of a coarse clock.
    statFaults.frozen.set(join(main, ".git", "config"), Date.now());
    const reader = gitHeadReader({ settleMs: 2_000, owners: async () => "owner" });
    expect((await read(reader, main)).head).toBeDefined();
    git(main, "config", "include.path", slash(included));
    clock.aheadMs += 3_000;
    expect((await read(reader, main)).head).toBeDefined();
    expect((await read(reader, main)).git).toBe(0);
    const refused = await broken(reader, main, included);
    expect(refused.expected).toBeUndefined();
    expect(refused.head).toBe(refused.expected);
  });

  it("asks Git again for the files includes name until every file of that answer has settled", async () => {
    const { root, main } = await repository();
    const [outer, inner] = [join(root, "outer.cfg"), join(root, "inner.cfg")];
    await probeFile(outer, "outer");
    await probeFile(inner, "inner");
    git(main, "config", "include.path", slash(outer));
    const clock = aheadClock();
    // The included file keeps one stamp through the write below, as in one tick of a coarse clock.
    statFaults.frozen.set(outer, Date.now());
    const reader = gitHeadReader({ settleMs: 2_000, owners: async () => "owner" });
    expect((await read(reader, main)).head).toBeDefined();
    await probeFile(outer, "outer", `[include]\n\tpath = ${slash(inner)}\n`);
    clock.aheadMs += 3_000;
    expect((await read(reader, main)).head).toBeDefined();
    expect((await read(reader, main)).git).toBe(0);
    const refused = await broken(reader, main, inner);
    expect(refused.expected).toBeUndefined();
    expect(refused.head).toBe(refused.expected);
  });

  it("reads a configuration file that is a link, as Git does, rather than asking Git on every read", async ({ skip }) => {
    const { root, main } = await repository();
    const [target, link] = [join(root, "dotfiles.cfg"), join(root, "global.cfg")];
    await writeFile(target, "[user]\n\tname = Rerail\n");
    try {
      await symlink(target, link);
    } catch (error) {
      // Windows makes a symbolic link only for an account that holds the privilege to.
      if (process.platform === "win32") skip();
      throw error;
    }
    setVariable("GIT_CONFIG_GLOBAL", link);
    const reader = filesReader();
    expect((await read(reader, main)).heads).toBe(1);
    expect((await read(reader, main)).git).toBe(0);
    const refused = await broken(reader, main, target);
    expect(refused.expected).toBeUndefined();
    expect(refused.head).toBe(refused.expected);
  });

  it("asks Git on every read while an include is spelled so that only Git can name its file", async () => {
    for (const spelling of [`~${userInfo().username}/.rerail-no-such-include.cfg`, "%(prefix)/rerail-no-such-include.cfg", ":(optional)no-such-include.cfg"]) {
      const { main } = await repository();
      git(main, "config", "include.path", spelling);
      const reader = filesReader();
      for (const time of [1, 2, 3]) {
        const answer = await read(reader, main);
        expect(answer.head, `${spelling}, read ${time}`).toBe(answer.expected);
        // Git is asked at every read. Git for Windows cannot expand another user's home, so there every Git command in
        // the checkout fails, and the reader asks `gitHead`'s `rev-parse` after its own.
        expect(answer.heads, `${spelling}, read ${time}`).toBeGreaterThan(0);
      }
    }
  });

  it("asks Git on every read while git var names a relative global file, which each checkout's Git opens in its own folder", async () => {
    const { main } = await repository();
    setVariable("GIT_CONFIG_GLOBAL", "relative-global.cfg");
    const reader = filesReader();
    for (const time of [1, 2]) {
      const answer = await read(reader, main);
      expect(answer.head, `read ${time}`).toBe(answer.expected);
      expect(answer.heads, `read ${time}`).toBe(1);
    }
    await writeFile(join(main, "relative-global.cfg"), "[broken\n");
    const refused = await read(reader, main);
    expect(refused.expected).toBeUndefined();
    expect(refused.head).toBe(refused.expected);
  });

  it("asks git var again at the next read after it failed, and then answers from Git's files", async () => {
    const { main } = await repository();
    const reader = filesReader();
    started.failing.add("GIT_CONFIG_SYSTEM");
    const failed = await read(reader, main);
    expect(failed.head).toBe(failed.expected);
    const again = await read(reader, main);
    expect(again.head).toBe(again.expected);
    expect(again.heads).toBe(1);
    expect((await read(reader, main)).git).toBe(0);
  });

  it("asks git var once where Git knows no such variable, and Git on every read", async () => {
    const { main } = await repository();
    started.oldGit = true;
    const reader = filesReader();
    const vars = (from: number) => started.git.slice(from).filter(args => args.includes("var")).length;
    for (const time of [1, 2, 3]) {
      const before = started.git.length;
      const answer = await read(reader, main);
      expect(answer.head, `read ${time}`).toBe(answer.expected);
      expect(answer.heads, `read ${time}`).toBe(1);
      expect(vars(before), `read ${time}`).toBe(time === 1 ? 1 : 0);
    }
  });

  /**
   * How a checkout's `gitdir:` line or `commondir` text names its folder through a link, `hop`, before a `..`: at first
   * `hop` leads to a folder beside it, so the text and Git name one folder; `elsewhere` is where it then leads, where Git
   * follows `hop` before the `..` and reads another Git folder, and cancelling `hop/..` as text names the first still.
   */
  const throughLinks: Array<[string, (root: string, main: string, worktree: string) => Promise<{ hop: string; elsewhere: string }>]> = [
    ["a commondir", async (root, main) => {
      const gitDir = join(main, ".git", "worktrees", "w");
      await mkdir(join(gitDir, "beside"));
      await symlink(join(gitDir, "beside"), join(gitDir, "hop"), "dir");
      await writeFile(join(gitDir, "commondir"), "hop/../../..\n");
      // Another common folder: the repository's as it is now, whose branches stay where they are, its `beside` copied too.
      await cp(join(main, ".git"), join(root, "other.git"), { recursive: true });
      return { hop: join(gitDir, "hop"), elsewhere: join(root, "other.git", "worktrees", "w", "beside") };
    }],
    ["a gitdir: line", async (root, main, worktree) => {
      const worktrees = join(main, ".git", "worktrees");
      await mkdir(join(worktrees, "beside"));
      await symlink(join(worktrees, "beside"), join(worktrees, "hop"), "dir");
      await writeFile(join(worktree, ".git"), `gitdir: ${worktrees}/hop/../w\n`);
      // Another Git folder for the worktree, on branch main, in the same repository.
      await cp(join(worktrees, "w"), join(root, "other", "w"), { recursive: true });
      await writeFile(join(root, "other", "w", "commondir"), `${join(main, ".git")}\n`);
      await writeFile(join(root, "other", "w", "HEAD"), "ref: refs/heads/main\n");
      await mkdir(join(root, "other", "beside"));
      return { hop: join(worktrees, "hop"), elsewhere: join(root, "other", "beside") };
    }],
  ];

  // Git for Windows opens the joined text, which Windows cleans as text: there `hop/..` cancels for Git too.
  it.skipIf(process.platform === "win32").each(throughLinks)("names HEAD as Git does where %s holds a link before a `..`, before and after the branch moves and the link leads elsewhere", async (_name, arrange) => {
    const { root, main, worktree } = await repository();
    const { hop, elsewhere } = await arrange(root, main, worktree);
    const reader = filesReader();
    const first = await read(reader, worktree);
    expect(first.head).toBe(git(worktree, "rev-parse", "HEAD"));
    expect(first.head).toBe(first.expected);
    await commit(worktree, "a = 2\n");
    const moved = await read(reader, worktree);
    expect(moved.head).toBe(git(worktree, "rev-parse", "HEAD"));
    await unlink(hop);
    await symlink(elsewhere, hop, "dir");
    const there = git(worktree, "rev-parse", "HEAD");
    // Git now reads the other Git folder, whose branch did not move with the commit above.
    expect(there).not.toBe(moved.head);
    for (const time of [1, 2]) {
      const answer = await read(reader, worktree);
      expect(answer.head, `read ${time} once the link leads elsewhere`).toBe(there);
    }
  });
});
