import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readdirSync, realpathSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  normalizeRemoteAddress,
  readCloneId,
  readRemoteAddress,
  repositoryIdentityLocation,
  resolveCloneId,
  type RepositoryIdentityLocation,
} from "../../../src/engine/catalog/repository-identity.js";
import { gitExecutable } from "../../../src/engine/git.js";

vi.mock("node:child_process", async importOriginal => {
  const actual = await importOriginal<typeof import("node:child_process")>();
  return { ...actual, spawnSync: vi.fn(actual.spawnSync) };
});

/** When set, the next `mkdirSync` of a `.rerail` folder loses a race: another process creates it first. */
const race = vi.hoisted(() => ({ armed: false }));
vi.mock("node:fs", async importOriginal => {
  const fs = await importOriginal<typeof import("node:fs")>();
  const mkdirSync = ((path: Parameters<typeof fs.mkdirSync>[0], options?: Parameters<typeof fs.mkdirSync>[1]) => {
    if (race.armed && String(path).endsWith(".rerail")) {
      race.armed = false;
      fs.mkdirSync(path, options);
      throw Object.assign(new Error(`EEXIST: file already exists, mkdir '${String(path)}'`), { code: "EEXIST" });
    }
    return fs.mkdirSync(path, options);
  }) as typeof fs.mkdirSync;
  return { ...fs, mkdirSync, default: { ...fs, mkdirSync } };
});

describe("normalizeRemoteAddress", () => {
  it("gives the ssh, https and differently cased addresses of one repository the same key", () => {
    const keys = [
      "git@github.com:acme/data-layer.git",
      "https://github.com/acme/data-layer.git",
      "https://github.com/Acme/data-layer",
    ].map(normalizeRemoteAddress);
    expect(keys).toEqual(Array(3).fill("remote:github.com/acme/data-layer"));
  });

  it("drops the user, the port, doubled and trailing slashes", () => {
    expect(normalizeRemoteAddress("ssh://git@github.com:22/acme//data-layer.git/")).toBe("remote:github.com/acme/data-layer");
    expect(normalizeRemoteAddress("https://user:token@gitlab.example.com:8443/team/web/")).toBe("remote:gitlab.example.com/team/web");
  });

  it("reads the scp form as host/path and keeps an ssh Host alias distinct", () => {
    expect(normalizeRemoteAddress("github.com:acme/web.git")).toBe("remote:github.com/acme/web");
    expect(normalizeRemoteAddress("github-work:acme/web")).toBe("remote:github-work/acme/web");
  });

  it("gives no key for a local path, a file URL or a Windows drive path", () => {
    expect(normalizeRemoteAddress("/srv/git/web.git")).toBeUndefined();
    expect(normalizeRemoteAddress("../web")).toBeUndefined();
    expect(normalizeRemoteAddress("file:///srv/git/web.git")).toBeUndefined();
    expect(normalizeRemoteAddress(String.raw`C:\repos\web`)).toBeUndefined();
    expect(normalizeRemoteAddress("C:/repos/web")).toBeUndefined();
  });
});

describe("readRemoteAddress", () => {
  const roots: string[] = [];
  afterEach(() => {
    for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  });

  const repository = (...remotes: [string, string][]): string => {
    const root = mkdtempSync(join(tmpdir(), "rerail-remote-"));
    roots.push(root);
    execFileSync("git", ["init", "-q", root]);
    for (const [name, url] of remotes) execFileSync("git", ["-C", root, "remote", "add", name, url]);
    return root;
  };

  it("prefers origin over any other remote", () => {
    const root = repository(["upstream", "https://github.com/acme/upstream.git"], ["origin", "git@git.example.com:acme/web.git"]);
    expect(readRemoteAddress(root)).toEqual({ name: "origin", url: "git@git.example.com:acme/web.git" });
  });

  it("uses the only remote when it is not called origin", () => {
    const root = repository(["upstream", "https://github.com/acme/web.git"]);
    expect(readRemoteAddress(root)).toEqual({ name: "upstream", url: "https://github.com/acme/web.git" });
  });

  it("gives none with two remotes and no origin, or with no remote", () => {
    expect(readRemoteAddress(repository(["a", "https://github.com/acme/a.git"], ["b", "https://github.com/acme/b.git"]))).toBeUndefined();
    expect(readRemoteAddress(repository())).toBeUndefined();
  });
});

describe("clone identity files", () => {
  const roots: string[] = [];
  afterEach(() => {
    for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  });
  const folder = (): string => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-identity-")));
    roots.push(root);
    return root;
  };

  it("refuses an identity that is a symlink, malformed, or kept in a symlinked .rerail, and never follows one", () => {
    const outside = folder();
    writeFileSync(join(outside, "secret"), "git:aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee\n");

    const linkedFile = folder();
    mkdirSync(join(linkedFile, ".rerail"));
    symlinkSync(join(outside, "secret"), join(linkedFile, ".rerail", "repository-id"));
    expect(readCloneId(linkedFile)).toBeUndefined();
    expect(() => resolveCloneId(linkedFile)).toThrow(/not a regular file/u);

    const malformed = folder();
    mkdirSync(join(malformed, ".rerail"));
    writeFileSync(join(malformed, ".rerail", "repository-id"), "../../etc\n");
    expect(readCloneId(malformed)).toBeUndefined();
    expect(() => resolveCloneId(malformed)).toThrow(/malformed/u);

    const linkedDirectory = folder();
    symlinkSync(outside, join(linkedDirectory, ".rerail"));
    expect(() => resolveCloneId(linkedDirectory)).toThrow(/not a directory/u);
    expect(readdirSync(outside)).toEqual(["secret"]);
  });

  it("creates its identity when another process creates .rerail at the same moment", () => {
    const root = folder();
    race.armed = true;
    const { cloneId } = resolveCloneId(root);
    expect(race.armed).toBe(false);
    expect(resolveCloneId(root).cloneId).toBe(cloneId);
  });

  it("creates a well-formed id once and reads the same id back", () => {
    const root = folder();
    const { cloneId } = resolveCloneId(root);
    expect(cloneId).toMatch(/^[0-9a-f-]{36}$/u);
    expect(resolveCloneId(root).cloneId).toBe(cloneId);
    expect(readCloneId(root)).toBe(cloneId);
  });
});

describe("a checkout's identity location", () => {
  const roots: string[] = [];
  afterEach(() => {
    for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
  });
  const folder = (): string => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-location-")));
    roots.push(root);
    return root;
  };
  const git = (cwd: string, ...args: string[]): string => execFileSync("git", [
    "-c", "protocol.file.allow=always", "-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args,
  ], { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] });
  const repository = (path: string): string => {
    mkdirSync(path, { recursive: true });
    git(path, "init", "-q", "-b", "main");
    git(path, "commit", "-q", "--allow-empty", "-m", "base");
    return path;
  };

  /** What Git itself answers for `folder` now, through a Git process of the test's own. */
  const gitsAnswer = (folder: string): RepositoryIdentityLocation => {
    let printed: string;
    try {
      printed = execFileSync("git", ["-C", folder, "rev-parse", "--show-toplevel", "--path-format=absolute", "--git-common-dir", "--git-dir"], {
        encoding: "utf8", stdio: ["ignore", "pipe", "ignore"],
      });
    } catch {
      return { git: false, workspaceRoot: folder, identityPath: join(folder, ".rerail", "repository-id"), main: true };
    }
    const [top, common, gitDirectory] = printed.trim().split("\n").map(line => resolve(line.trim())) as [string, string, string];
    return { git: true, workspaceRoot: top, identityPath: join(common, "rerail-repository-id"), main: common === gitDirectory };
  };

  /** The Git processes Rerail started and waited for (`spawnSync`) while `run` ran. */
  const syncGitProcesses = (run: () => void): number => {
    const executable = gitExecutable();
    const before = vi.mocked(spawnSync).mock.calls.length;
    run();
    return vi.mocked(spawnSync).mock.calls.slice(before).filter(([file]) => file === executable).length;
  };

  /** Looks at `folder` `times` times, each answer checked against Git's own; the Git processes Rerail started. */
  const looks = (folder: string, times = 3): number => syncGitProcesses(() => {
    for (let call = 0; call < times; call += 1) expect(repositoryIdentityLocation(folder)).toEqual(gitsAnswer(folder));
  });

  it("asks Git once for ten looks at a main checkout, a folder in it, a linked worktree and a submodule", () => {
    const root = folder();
    const main = repository(join(root, "main"));
    const library = repository(join(root, "library"));
    git(main, "submodule", "add", "-q", library, "vendor/library");
    git(main, "commit", "-q", "-m", "submodule");
    mkdirSync(join(main, "src"));
    const worktree = join(root, "linked");
    git(main, "worktree", "add", "-q", "-b", "linked", worktree);

    for (const checkout of [main, join(main, "src"), worktree, join(main, "vendor", "library")]) {
      expect(looks(checkout, 10), checkout).toBe(1);
    }
    const ofMain = repositoryIdentityLocation(main);
    expect(repositoryIdentityLocation(join(main, "src"))).toEqual(ofMain);
    expect(repositoryIdentityLocation(worktree)).toMatchObject({ main: false, identityPath: ofMain.identityPath });
    const ofSubmodule = repositoryIdentityLocation(join(main, "vendor", "library"));
    expect(ofSubmodule.main).toBe(true);
    expect(ofSubmodule.identityPath).not.toBe(ofMain.identityPath);
  });

  it("answers as Git does after a worktree is removed and added again at its path, from its repository or another", () => {
    const root = folder();
    const first = repository(join(root, "first"));
    const second = repository(join(root, "second"));
    const path = join(root, "w");
    git(first, "worktree", "add", "-q", "-b", "one", path);
    looks(path);

    // Its Git folder removed under it, its `.git` file left: Git no longer takes the folder for a checkout.
    rmSync(join(first, ".git", "worktrees", "w"), { recursive: true, force: true });
    looks(path);
    expect(repositoryIdentityLocation(path).git).toBe(false);
    rmSync(path, { recursive: true, force: true });
    looks(path);
    git(first, "worktree", "add", "-q", "-b", "two", path);
    looks(path);
    // Removed and added again with no look between: the same `.git` text and Git folder, so the same answer.
    git(first, "worktree", "remove", path);
    git(first, "worktree", "add", "-q", "-b", "three", path);
    looks(path);
    git(first, "worktree", "remove", path);
    git(second, "worktree", "add", "-q", "-b", "four", path);
    looks(path);
    expect(repositoryIdentityLocation(path).identityPath).toBe(repositoryIdentityLocation(second).identityPath);

    // A clone of its own at the same path, then a repository made in a folder inside it.
    git(second, "worktree", "remove", path);
    repository(path);
    looks(path);
    expect(repositoryIdentityLocation(path).main).toBe(true);
    expect(repositoryIdentityLocation(path).identityPath).not.toBe(repositoryIdentityLocation(second).identityPath);
    const nested = join(path, "nested");
    mkdirSync(nested);
    looks(nested);
    expect(repositoryIdentityLocation(nested)).toEqual(repositoryIdentityLocation(path));
    repository(nested);
    looks(nested);
    expect(repositoryIdentityLocation(nested).identityPath).not.toBe(repositoryIdentityLocation(path).identityPath);
  });

  it("answers as Git does when a link on the way to a checkout comes to lead to another repository", () => {
    const root = folder();
    const one = repository(join(root, "one"));
    const two = repository(join(root, "two"));
    const link = join(root, "link");
    // A junction on Windows, which needs no privilege; elsewhere the type is ignored.
    symlinkSync(one, link, "junction");
    looks(link);
    unlinkSync(link);
    symlinkSync(two, link, "junction");
    looks(link);
  });

  it("answers as Git does when core.bare takes the work tree away and gives it back", () => {
    const checkout = repository(join(folder(), "web"));
    looks(checkout);
    git(checkout, "config", "core.bare", "true");
    looks(checkout);
    expect(repositoryIdentityLocation(checkout).git).toBe(false);
    git(join(checkout, ".git"), "config", "core.bare", "false");
    looks(checkout);
    expect(repositoryIdentityLocation(checkout).git).toBe(true);
  });

  it("reads a remote that changes after the checkout was located, and asks Git for no location again", () => {
    const checkout = repository(join(folder(), "web"));
    git(checkout, "remote", "add", "origin", "https://github.com/acme/web.git");
    const location = repositoryIdentityLocation(checkout);
    expect(readRemoteAddress(location.workspaceRoot)).toEqual({ name: "origin", url: "https://github.com/acme/web.git" });

    git(checkout, "remote", "set-url", "origin", "git@github.com:acme/renamed.git");
    git(checkout, "remote", "add", "upstream", "https://github.com/acme/upstream.git");
    expect(looks(checkout)).toBe(0);
    expect(readRemoteAddress(repositoryIdentityLocation(checkout).workspaceRoot)).toEqual({ name: "origin", url: "git@github.com:acme/renamed.git" });
    git(checkout, "remote", "remove", "origin");
    expect(readRemoteAddress(repositoryIdentityLocation(checkout).workspaceRoot)).toEqual({ name: "upstream", url: "https://github.com/acme/upstream.git" });
  });

  it("reads the legacy id from Git's config only for a checkout that has no identity file yet", () => {
    const checkout = repository(join(folder(), "web"));
    git(checkout, "config", "rerail.repository-id", "legacy-id.1");
    const resolved: string[] = [];
    expect(syncGitProcesses(() => {
      for (let call = 0; call < 10; call += 1) resolved.push(resolveCloneId(checkout).cloneId);
    })).toBe(2);
    expect(resolved).toEqual(Array(10).fill("legacy-id.1"));
  });
});
