import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { GitIgnoreMatchers } from "../../../src/engine/native/git-ignore-matchers.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const folders: string[] = [];
afterEach(async () => {
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  for (const folder of folders.splice(0)) await removeFolder(folder);
});

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, stdio: "ignore" });
}

/**
 * A committed repository holding a `.gitignore` of `gen/` and a linked worktree beside it, with a home of its own, where
 * the engine and the matchers both find Git's default excludes file.
 */
async function checkout() {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-ignore-matchers-")));
  folders.push(root);
  const main = join(root, "main");
  await mkdir(main);
  await writeFile(join(main, ".gitignore"), "gen/\n");
  git(main, "init", "--quiet");
  git(main, "add", ".");
  git(main, "commit", "--quiet", "-m", "seed");
  git(main, "worktree", "add", "--quiet", "--detach", join(root, "linked"));
  const home = join(root, "home");
  await mkdir(join(home, "config", "git"), { recursive: true });
  vi.stubEnv("HOME", home);
  vi.stubEnv("XDG_CONFIG_HOME", join(home, "config"));
  return { root, main, home, linked: await realpath(join(root, "linked")), excludes: join(home, "config", "git", "ignore") };
}

/** A path as a configuration value: forward slashes, which Git reads on every platform. */
function slashed(path: string): string {
  return path.split("\\").join("/");
}

/** A router as a user of the matchers: one object for each name. */
const users = new Map<string, object>();
function user(name: string): object {
  const known = users.get(name) ?? { name };
  users.set(name, known);
  return known;
}

describe("GitIgnoreMatchers", () => {
  it("shares one matcher for a checkout while nothing it read changed, a configuration change that leaves its rules alone included", async () => {
    const { main } = await checkout();
    const opened = vi.spyOn(loadNativeBinding(), "openGitIgnoreMatcher");
    const matchers = new GitIgnoreMatchers();
    const first = matchers.matcher(main, user("first"));
    expect(first?.ignored("gen/a.py", false)).toBe(true);
    expect(matchers.matcher(main, user("second"))).toBe(first);
    // A configuration change that names no other excludes file and folds no case (an upstream `git push -u` sets).
    git(main, "config", "branch.main.remote", "origin");
    expect(matchers.matcher(main, user("third"))).toBe(first);
    expect(opened).toHaveBeenCalledTimes(1);
    for (const name of ["first", "second", "third"]) matchers.release(user(name));
    expect(matchers.size).toBe(0);
  });

  it("opens a fresh matcher for a router when the shared one cannot say whether what it read changed", async () => {
    const { main } = await checkout();
    const binding = loadNativeBinding();
    let opens = 0;
    const matchers = new GitIgnoreMatchers(root => {
      opens += 1;
      const matcher = binding.openGitIgnoreMatcher(root);
      return matcher === null ? null : {
        ignored: (path, directory) => matcher.ignored(path, directory),
        ignoredBy: (path, directory) => matcher.ignoredBy(path, directory),
        sourcesUnchanged: () => { throw new Error("cannot read the Git configuration"); },
      };
    });
    const first = matchers.matcher(main, user("first"));
    expect(matchers.matcher(main, user("second"))).not.toBe(first);
    expect(opens).toBe(2);
  });

  it("gives a router that asks after the checkout's .gitignore changed a matcher that follows it, and leaves an earlier one's as it was", async () => {
    const { main } = await checkout();
    const matchers = new GitIgnoreMatchers();
    const before = matchers.matcher(main, user("before"));
    expect(before?.ignored("gen/a.py", false)).toBe(true);

    await writeFile(join(main, ".gitignore"), "gen2/\n");
    const after = matchers.matcher(main, user("after"));
    expect([after?.ignored("gen/a.py", false), after?.ignored("gen2/a.py", false)]).toEqual([false, true]);
    // A router keeps the matcher it took for its life, as it kept one of its own.
    expect([before?.ignored("gen/b.py", false), before?.ignored("gen2/b.py", false)]).toEqual([true, false]);
    expect(matchers.size).toBe(2);
    matchers.release(user("before"));
    expect(matchers.size).toBe(1);
  });

  it("follows a nested .gitignore added, changed and removed before a router asks, where the shared matcher last read that folder", async () => {
    const { linked } = await checkout();
    const matchers = new GitIgnoreMatchers();
    const ask = (name: string, path: string) => matchers.matcher(linked, user(name))?.ignored(path, false);
    // The first router's question leaves the matcher's walk in pkg/n1, before pkg holds a .gitignore.
    expect(ask("first", "pkg/n1/x.py")).toBe(false);

    await mkdir(join(linked, "pkg"));
    await writeFile(join(linked, "pkg", ".gitignore"), "n1/\n");
    expect([ask("added", "pkg/n1/y.py"), ask("added", "pkg/n2/y.py")]).toEqual([true, false]);
    await writeFile(join(linked, "pkg", ".gitignore"), "n2/\n");
    expect([ask("changed", "pkg/n2/y.py"), ask("changed", "pkg/n1/y.py")]).toEqual([true, false]);
    // Asked from pkg/n1 again, then pkg/.gitignore goes.
    expect(ask("changed", "pkg/n1/z.py")).toBe(false);
    await rm(join(linked, "pkg", ".gitignore"));
    expect([ask("removed", "pkg/n1/z.py"), ask("removed", "pkg/n2/z.py")]).toEqual([false, false]);
  });

  it("follows info/exclude, Git's default excludes file and the excludes file the configuration names, in a linked worktree", async () => {
    const { main, linked, excludes, root } = await checkout();
    const matchers = new GitIgnoreMatchers();
    const ask = (name: string, path: string) => matchers.matcher(linked, user(name))?.ignored(path, false);
    expect(ask("first", "local.py")).toBe(false);

    await writeFile(join(main, ".git", "info", "exclude"), "local.py\n");
    expect(ask("exclude", "local.py")).toBe(true);
    await writeFile(excludes, "home.py\n");
    expect(ask("default", "home.py")).toBe(true);
    const named = join(root, "named-ignore");
    await writeFile(named, "");
    git(main, "config", "core.excludesFile", named);
    expect(ask("configured", "named.py")).toBe(false);
    await writeFile(named, "named.py\n");
    expect(ask("named", "named.py")).toBe(true);
    // Git folds the case of patterns once core.ignoreCase holds (on Windows from `git init` already).
    git(main, "config", "core.ignoreCase", "true");
    expect(ask("folded", "GEN/x.py")).toBe(true);
  });

  it("follows an excludes file the global configuration names in a file it includes, and that file's changes", async () => {
    const { root, home, linked } = await checkout();
    const matchers = new GitIgnoreMatchers();
    const ask = (name: string, path: string) => matchers.matcher(linked, user(name))?.ignored(path, false);
    expect(ask("first", "inc/a.py")).toBe(false);

    const named = join(root, "included-ignore");
    await writeFile(named, "inc/\n");
    await writeFile(join(home, "inc.gitconfig"), `[core]\n\texcludesFile = ${slashed(named)}\n`);
    git(linked, "config", "--global", "include.path", slashed(join(home, "inc.gitconfig")));
    expect(ask("included", "inc/a.py")).toBe(true);
    await writeFile(named, "inc2/\n");
    expect([ask("changed", "inc/a.py"), ask("changed", "inc2/a.py")]).toEqual([false, true]);
  });

  it("follows an excludes file a conditional include of the global configuration names", async () => {
    const { root, home, linked } = await checkout();
    const matchers = new GitIgnoreMatchers();
    const ask = (name: string, path: string) => matchers.matcher(linked, user(name))?.ignored(path, false);
    expect(ask("first", "cond/a.py")).toBe(false);

    const named = join(root, "conditional-ignore");
    await writeFile(named, "cond/\n");
    await writeFile(join(home, "cond.gitconfig"), `[core]\n\texcludesFile = ${slashed(named)}\n`);
    // Every Git folder below the test's folder meets the condition, the linked worktree's among them.
    git(linked, "config", "--global", `includeIf.gitdir:${slashed(root)}/.path`, slashed(join(home, "cond.gitconfig")));
    expect(ask("conditional", "cond/a.py")).toBe(true);
  });

  it("follows the file a configuration value names with a comment after it, and that file's changes", async () => {
    const { root, home, linked } = await checkout();
    const matchers = new GitIgnoreMatchers();
    const ask = (name: string, path: string) => matchers.matcher(linked, user(name))?.ignored(path, false);
    const named = join(root, "commented-ignore");
    await writeFile(named, "c1/\n");
    // A hand-edited line: Git reads the value up to the comment.
    await writeFile(join(home, ".gitconfig"), `[core]\n\texcludesFile = ${slashed(named)} ; mine\n`);
    expect([ask("first", "c1/a.py"), ask("first", "c2/a.py")]).toEqual([true, false]);
    await writeFile(named, "c2/\n");
    expect([ask("changed", "c1/a.py"), ask("changed", "c2/a.py")]).toEqual([false, true]);
  });
});
