import { execFileSync } from "node:child_process";
import { chmod, mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  findGit, gitBehindLauncher, gitBranchTips, gitCandidates, gitCommitsBetween, gitConfigFiles, gitCrlfPaths, gitExcludeFiles, gitExecutable, gitHead, gitHome,
  gitInstalled, GitUnavailableError, headMovedOnlyByCommits, readGitBlobIds, readGitBlobs,
  runGit, searchGitFrom,
} from "../../src/engine/git.js";
import { honoursNoLazyFetch, partialCloneLacks } from "../support/partial-clone.js";

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, encoding: "utf8" });
}

async function repository(files: Record<string, string>): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-git-blobs-"));
  roots.push(root);
  git(root, "init", "--quiet");
  for (const [path, text] of Object.entries(files)) await writeFile(join(root, path), text);
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  return root;
}

/** A `--filter=blob:none` clone of a one-file repository, and the id of the blob it does not hold. */
async function partialClone(): Promise<{ clone: string; oid: string }> {
  const origin = await repository({ "a.py": "print(1)\n" });
  git(origin, "config", "uploadpack.allowFilter", "true");
  const clone = join(origin, "..", `${origin.split("/").pop()}-clone`);
  roots.push(clone);
  git(origin, "clone", "--quiet", "--filter=blob:none", "--no-checkout", `file://${origin}`, clone);
  return { clone, oid: git(clone, "rev-parse", "HEAD:a.py").trim() };
}

describe("readGitBlobs", () => {
  it("keeps every blob it was asked for when another in the batch is larger than the limit", async () => {
    const root = await repository({ "big.py": "x".repeat(5_000), "small.py": "print(1)\n" });
    await writeFile(join(root, "small.py"), "print(2)\n");
    git(root, "add", "small.py");
    const blobs = await readGitBlobs(root, [":./big.py", ":./small.py", "HEAD:./small.py", ":./missing.py"], { maxBlobBytes: 1_000 });
    expect(Object.fromEntries([...blobs].map(([revision, blob]) => [revision, blob.toString("utf8")])))
      .toEqual({ ":./small.py": "print(2)\n", "HEAD:./small.py": "print(1)\n" });
  });

  it.skipIf(!honoursNoLazyFetch())("never fetches a blob a partial clone does not hold", async () => {
    const { clone, oid } = await partialClone();
    expect(await readGitBlobs(clone, ["HEAD:./a.py"], { maxBlobBytes: 1_000 })).toEqual(new Map());
    expect(partialCloneLacks(clone, oid)).toBe(true);
  });
});

describe("readGitBlobIds", () => {
  it("names each blob a revision or object id resolves to, and nothing for a missing one or a tree, whatever its path holds", async () => {
    const root = await repository({ "a b.py": "1\n", "x blob": "2\n" });
    await mkdir(join(root, "dir"));
    await writeFile(join(root, "dir", "c.py"), "3\n");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "more");
    const spaced = git(root, "rev-parse", "HEAD:a b.py").trim();
    const ids = await readGitBlobIds(root, [
      "HEAD:./a b.py", "HEAD:./x blob", "HEAD:./dir", "HEAD:./gone blob", spaced, "0".repeat(40), "HEAD:./dir/c.py",
    ]);
    expect(Object.fromEntries(ids)).toEqual({
      "HEAD:./a b.py": spaced,
      "HEAD:./x blob": git(root, "rev-parse", "HEAD:x blob").trim(),
      [spaced]: spaced,
      "HEAD:./dir/c.py": git(root, "rev-parse", "HEAD:dir/c.py").trim(),
    });
  });

  it("names nothing outside a repository", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-git-ids-"));
    roots.push(root);
    await expect(readGitBlobIds(root, ["HEAD:./a.py"])).resolves.toEqual(new Map());
  });
});

describe("gitBranchTips and gitCommitsBetween", () => {
  it("lists each commit a branch or remote-tracking branch points at once, and no tag", async () => {
    const root = await repository({ "a.py": "1\n" });
    const seed = (await gitHead(root))!;
    git(root, "branch", "same");
    git(root, "tag", "v1");
    git(root, "switch", "--quiet", "-c", "other");
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    const other = (await gitHead(root))!;
    git(root, "update-ref", "refs/remotes/origin/main", seed);
    git(root, "tag", "v2", other);
    git(root, "switch", "--quiet", "--detach", seed);
    git(root, "commit", "--quiet", "--allow-empty", "-m", "detached");
    expect((await gitBranchTips(root)).sort()).toEqual([seed, other].sort());
  });

  it("lists the commits a move added, and none where Git cannot say", async () => {
    const root = await repository({ "a.py": "1\n" });
    const seed = (await gitHead(root))!;
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    git(root, "commit", "--quiet", "--allow-empty", "-m", "three");
    const head = (await gitHead(root))!;
    expect(await gitCommitsBetween(root, seed, head)).toEqual([head, git(root, "rev-parse", "HEAD~1").trim()]);
    expect(await gitCommitsBetween(root, head, seed)).toEqual([]);
    expect(await gitCommitsBetween(root, "0".repeat(40), head)).toEqual([]);
  });
});

describe("runGit", () => {
  it.skipIf(!honoursNoLazyFetch())("never fetches an object a partial clone does not hold", async () => {
    const { clone, oid } = await partialClone();
    expect(runGit(clone, ["cat-file", "-p", oid]).status).not.toBe(0);
    expect(partialCloneLacks(clone, oid)).toBe(true);
  });
});

describe("gitHead and headMovedOnlyByCommits", () => {
  it("names no commit outside a repository, or where nothing is committed yet", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-git-head-"));
    roots.push(root);
    await expect(gitHead(root)).resolves.toBeUndefined();
    git(root, "init", "--quiet");
    await expect(gitHead(root)).resolves.toBeUndefined();
  });

  it("tells commits from anything else that moved HEAD since a commit", async () => {
    const root = await repository({ "a.py": "1\n" });
    const seed = (await gitHead(root))!;
    git(root, "commit", "--quiet", "--allow-empty", "-m", "two");
    git(root, "commit", "--quiet", "--allow-empty", "--amend", "-m", "two again");
    expect(await headMovedOnlyByCommits(root, seed)).toBe(true);
    const committed = (await gitHead(root))!;
    git(root, "reset", "--quiet", "--hard", seed);
    expect(await headMovedOnlyByCommits(root, committed)).toBe(false);
    // A commit after the move does not hide it.
    git(root, "commit", "--quiet", "--allow-empty", "-m", "three");
    expect(await headMovedOnlyByCommits(root, committed)).toBe(false);
  });

  it("goes by HEAD's newest move for a checkout not seen before, and takes a commit it cannot find as moved", async () => {
    const root = await repository({ "a.py": "1\n" });
    expect(await headMovedOnlyByCommits(root, undefined)).toBe(true);
    git(root, "switch", "--quiet", "-c", "other");
    expect(await headMovedOnlyByCommits(root, undefined)).toBe(false);
    expect(await headMovedOnlyByCommits(root, "0".repeat(40))).toBe(false);
  });
});

describe("gitExcludeFiles", () => {
  afterEach(() => { vi.unstubAllEnvs(); });

  /** A repository in a fresh home whose Git reads no global or system configuration. */
  async function isolatedRepository(): Promise<{ root: string; home: string }> {
    const home = await realpath(await mkdtemp(join(tmpdir(), "rerail-git-exclude-")));
    roots.push(home);
    vi.stubEnv("HOME", home);
    vi.stubEnv("XDG_CONFIG_HOME", "");
    vi.stubEnv("GIT_CONFIG_GLOBAL", join(home, "no-gitconfig"));
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    const root = join(home, "repo");
    await mkdir(root);
    git(root, "init", "--quiet");
    git(root, "commit", "--quiet", "--allow-empty", "-m", "seed");
    return { root, home };
  }

  it("names the checkout's info/exclude and Git's default excludes file while core.excludesFile is unset", async () => {
    const { root, home } = await isolatedRepository();
    expect(await gitExcludeFiles(root)).toEqual([join(root, ".git", "info", "exclude"), join(home, ".config", "git", "ignore")]);
    vi.stubEnv("XDG_CONFIG_HOME", join(home, "xdg"));
    expect((await gitExcludeFiles(root))[1]).toBe(join(home, "xdg", "git", "ignore"));
  });

  it("follows core.excludesFile, with `~` read as the home, and gives a linked worktree the common info/exclude", async () => {
    const { root, home } = await isolatedRepository();
    git(root, "config", "core.excludesFile", "~/ignores");
    git(root, "worktree", "add", "--quiet", join(home, "linked"));
    expect(await gitExcludeFiles(join(home, "linked"))).toEqual([join(root, ".git", "info", "exclude"), join(home, "ignores")]);
  });

  it("names none outside a Git checkout", async () => {
    const { home } = await isolatedRepository();
    expect(await gitExcludeFiles(home)).toEqual([]);
  });

  it("names the configuration files that can set core.excludesFile: the repository's, the worktree's and the global ones", async () => {
    const { root, home } = await isolatedRepository();
    vi.stubEnv("GIT_CONFIG_GLOBAL", "");
    expect(await gitConfigFiles(root)).toEqual([
      join(root, ".git", "config"), join(root, ".git", "config.worktree"), join(home, ".gitconfig"), join(home, ".config", "git", "config"),
    ]);
    vi.stubEnv("GIT_CONFIG_GLOBAL", join(home, "global"));
    expect((await gitConfigFiles(root)).slice(2)).toEqual([join(home, "global")]);
    expect(await gitConfigFiles(home)).toEqual([]);
  });

  it("finds the global files in Git's own home, which on Windows without HOME is HOMEDRIVE and HOMEPATH", async () => {
    const { root, home } = await isolatedRepository();
    vi.stubEnv("GIT_CONFIG_GLOBAL", "");
    const share = join(home, "share");
    await mkdir(share);
    const globals = [join(share, ".gitconfig"), join(share, ".config", "git", "config")];
    // Git's home, not the account's: HOME here names another folder than the one homedir() answers.
    expect((await gitConfigFiles(root, { ...process.env, HOME: share })).slice(2)).toEqual(globals);
    if (process.platform === "win32") {
      const { HOME: _home, ...withoutHome } = process.env;
      const drive = share.slice(0, 2);
      expect((await gitConfigFiles(root, { ...withoutHome, HOMEDRIVE: drive, HOMEPATH: share.slice(2) }, "win32")).slice(2)).toEqual(globals);
    }
  });
});

describe("gitBehindLauncher", () => {
  it("runs Git for Windows' own git.exe, not the launcher that starts it as a child a timeout would leave running", () => {
    const git = String.raw`C:\Program Files\Git`;
    const launcher = String.raw`${git}\cmd\git.exe`;
    const search = { platform: "win32" as const, home: String.raw`C:\Users\me`, path: undefined, cwd: undefined };
    const installed = (...paths: string[]) => {
      const files = [launcher, String.raw`${git}\etc\gitconfig`, ...paths];
      return { exists: (path: string) => files.includes(path), runnable: (path: string) => files.includes(path), realpath: (path: string) => path, developerDir: () => undefined };
    };
    const real = String.raw`${git}\mingw64\bin\git.exe`;
    expect(gitBehindLauncher(launcher, search, installed(real))).toBe(real);
    expect(gitBehindLauncher(String.raw`${git}\bin\git.exe`, search, installed(real))).toBe(real);
    expect(gitBehindLauncher(launcher, search, installed(String.raw`${git}\clangarm64\bin\git.exe`))).toBe(String.raw`${git}\clangarm64\bin\git.exe`);
    // The real one is itself, and a Git laid out otherwise (a scoop shim, a launcher without its Git) is run as found.
    expect(gitBehindLauncher(real, search, installed(real))).toBe(real);
    expect(gitBehindLauncher(String.raw`D:\scoop\shims\git.exe`, search, installed(real))).toBe(String.raw`D:\scoop\shims\git.exe`);
    expect(gitBehindLauncher(launcher, search, installed())).toBe(launcher);
  });

  it.runIf(process.platform === "win32")("is what Rerail runs on this Windows machine", () => {
    expect(gitExecutable()).toBe(gitBehindLauncher(gitExecutable()));
    expect(gitExecutable().toLowerCase()).not.toMatch(/\\cmd\\git\.exe$/u);
  });
});

describe("gitCrlfPaths", () => {
  it("names the files a checkout writes with CRLF line breaks, as Git's attributes and configuration decide it", async () => {
    const root = await repository({ "a.py": "a\n", "b.bin": "b\n", "c.txt": "c\n", "d.md": "d\n" });
    git(root, "config", "core.autocrlf", "false");
    git(root, "config", "core.eol", "native");
    await writeFile(join(root, ".git", "info", "attributes"), "*.py text\n*.bin -text eol=crlf\n*.txt eol=crlf\n");
    const paths = ["a.py", "b.bin", "c.txt", "d.md"];
    // A text file takes core.eol, whose default (native) is CRLF on Windows alone; -text is never converted.
    expect([...await gitCrlfPaths(root, paths, "win32")].sort()).toEqual(["a.py", "c.txt"]);
    expect([...await gitCrlfPaths(root, paths, "linux")]).toEqual(["c.txt"]);
    // core.autocrlf=true converts every file with no text attribute too; input converts none on checkout.
    git(root, "config", "core.autocrlf", "true");
    expect([...await gitCrlfPaths(root, paths, "linux")].sort()).toEqual(["a.py", "c.txt", "d.md"]);
    git(root, "config", "core.autocrlf", "input");
    expect([...await gitCrlfPaths(root, paths, "win32")]).toEqual(["c.txt"]);
  });
});

describe("gitHome", () => {
  it("on Windows without HOME takes HOMEDRIVE and HOMEPATH when that folder exists, as Git for Windows does", () => {
    const share = { HOMEDRIVE: "H:", HOMEPATH: "\\" };
    expect(gitHome(share, "win32", path => path === "H:\\")).toBe("H:\\");
    expect(gitHome(share, "win32", () => false)).toBe(homedir());
    expect(gitHome({ ...share, HOME: "C:\\Users\\me" }, "win32", () => true)).toBe("C:\\Users\\me");
    expect(gitHome(share, "linux", () => true)).toBe(homedir());
  });
});

describe("gitCandidates", () => {
  const disk = (files: readonly string[], links: Record<string, string> = {}) => ({
    exists: (path: string) => files.includes(path),
    realpath: (path: string) => Object.entries(links).reduce((real, [link, target]) => real.startsWith(link) ? target + real.slice(link.length) : real, path),
  });

  it("tries PATH first, in its order, then the usual install locations, each once", () => {
    const search = { platform: "linux" as const, home: "/home/me", path: "/home/me/.local/share/mise/shims:/opt/local/bin:/usr/bin", cwd: "/tmp" };
    expect(gitCandidates(search, disk([]))).toEqual([
      "/home/me/.local/share/mise/shims/git",
      "/opt/local/bin/git",
      "/usr/bin/git",
      "/usr/local/bin/git",
      "/opt/homebrew/bin/git",
      "/home/linuxbrew/.linuxbrew/bin/git",
      "/run/current-system/sw/bin/git",
      "/home/me/.nix-profile/bin/git",
    ]);
  });

  it("skips PATH entries a repository could control: relative, inside the repository it runs in, or in node_modules", () => {
    const search = {
      platform: "darwin" as const,
      home: "/Users/me",
      // Empty and `.` mean the current folder; bin is direnv's PATH_add from the repository's .envrc.
      path: ":.:bin:/Users/me/code/app/bin:/Users/me/code/app/node_modules/.bin:/Users/me/code/node_modules/.bin:/Users/me/code/app-tools/bin:/opt/homebrew/bin",
      cwd: "/Users/me/code/app/src",
    };
    expect(gitCandidates(search, disk(["/Users/me/code/app/.git"])).slice(0, 2)).toEqual([
      "/Users/me/code/app-tools/bin/git",
      "/opt/homebrew/bin/git",
    ]);
  });

  it("skips an entry that reaches the repository through a symlink, either way round", () => {
    // macOS's /var is /private/var: the folder Rerail runs in reads as /private/var, PATH as /var.
    const search = { platform: "darwin" as const, home: "/Users/me", path: "/var/t/app/bin:/Users/me/tools:/usr/bin", cwd: "/private/var/t/app" };
    const links = { "/var/": "/private/var/", "/Users/me/tools": "/private/var/t/app/bin" };
    expect(gitCandidates(search, disk(["/private/var/t/app/.git"], links))[0]).toBe("/usr/bin/git");
  });

  it("on Windows takes git.exe from absolute entries only, unquoted, with the same skips in any case", () => {
    // A repository's environment (direnv, a devcontainer) can set LOCALAPPDATA as easily as PATH.
    vi.stubEnv("LOCALAPPDATA", String.raw`C:\repo\evil`);
    try {
      const search = {
        platform: "win32" as const,
        home: String.raw`C:\Users\me`,
        path: String.raw`"C:\Program Files\Git\cmd";.;\tools;C:\Code\App\Bin;C:\code\web\NODE_MODULES\.bin;D:\scoop\shims`,
        cwd: String.raw`c:\code\app`,
      };
      expect(gitCandidates(search, disk([String.raw`c:\code\app\.git`]))).toEqual([
        String.raw`C:\Program Files\Git\cmd\git.exe`,
        String.raw`D:\scoop\shims\git.exe`,
        String.raw`C:\Program Files (x86)\Git\cmd\git.exe`,
        String.raw`C:\Users\me\AppData\Local\Programs\Git\cmd\git.exe`,
        String.raw`C:\Users\me\scoop\apps\git\current\cmd\git.exe`,
      ]);
    } finally {
      vi.unstubAllEnvs();
    }
  });

  it("on Windows also looks in Program Files on the drive of the account's home", () => {
    const search = { platform: "win32" as const, home: String.raw`D:\Users\a`, path: undefined, cwd: undefined };
    expect(gitCandidates(search, disk([]))).toEqual(expect.arrayContaining([
      String.raw`D:\Program Files\Git\cmd\git.exe`,
      String.raw`D:\Program Files (x86)\Git\cmd\git.exe`,
    ]));
  });

  it("keeps every absolute entry outside node_modules when the folder it runs in is in no repository", () => {
    const search = { platform: "linux" as const, home: "/home/me", path: "/home/me/bin", cwd: "/home/me" };
    expect(gitCandidates(search, disk([]))[0]).toBe("/home/me/bin/git");
  });

  it("skips the PATH entries of the repository it was told to search from, wherever it runs", async () => {
    // The daemon runs in the Rerail home and serves the repository it was started for (`--workspace-root`).
    const repository = await realpath(await mkdtemp(join(tmpdir(), "rerail-git-served-")));
    roots.push(repository);
    await mkdir(join(repository, ".git"));
    const tools = await realpath(await mkdtemp(join(tmpdir(), "rerail-git-tools-")));
    roots.push(tools);
    const program = process.platform === "win32" ? "git.exe" : "git";
    vi.stubEnv("PATH", [join(repository, "bin"), tools].join(delimiter));
    try {
      expect(gitCandidates()).toContain(join(repository, "bin", program));
      searchGitFrom(repository);
      expect(gitCandidates()).not.toContain(join(repository, "bin", program));
      expect(gitCandidates()[0]).toBe(join(tools, program));
    } finally {
      searchGitFrom(undefined);
      vi.unstubAllEnvs();
    }
  });

  it("still lists PATH and the install locations from a working folder that was deleted", async () => {
    const gone = await mkdtemp(join(tmpdir(), "rerail-git-gone-"));
    const before = process.cwd();
    process.chdir(gone);
    try {
      await rm(gone, { recursive: true });
      expect(() => process.cwd()).toThrow();
      vi.stubEnv("PATH", "/opt/local/bin");
      expect(gitCandidates()).toContain("/opt/local/bin/git");
    } finally {
      vi.unstubAllEnvs();
      process.chdir(before);
    }
  });
});

describe("findGit", () => {
  /** A disk holding `files`, each runnable, that counts the realpath calls it answers. */
  const counted = (files: readonly string[]) => {
    const realpaths: string[] = [];
    return {
      realpaths,
      exists: (path: string) => files.includes(path),
      runnable: (path: string) => files.includes(path),
      realpath: (path: string) => {
        realpaths.push(path);
        return path;
      },
      developerDir: () => undefined,
    };
  };
  const entries = Array.from({ length: 40 }, (_, index) => `/opt/tool-${index}/bin`);

  it("stops at the first Git that runs, without resolving the PATH entries after it", () => {
    const search = { platform: "linux" as const, home: "/home/me", path: ["/opt/first/bin", "/opt/git/bin", ...entries].join(":"), cwd: "/home/me/app" };
    const disk = counted(["/home/me/app/.git", "/opt/git/bin/git"]);
    expect(findGit(search, disk).path).toBe("/opt/git/bin/git");
    // The repository it runs in, then each entry up to the Git found.
    expect(disk.realpaths).toEqual(["/home/me/app", "/opt/first/bin", "/opt/git/bin"]);
    // Outside any repository no entry needs its real path.
    const outside = counted(["/opt/git/bin/git"]);
    expect(findGit({ ...search, cwd: "/home/me" }, outside).path).toBe("/opt/git/bin/git");
    expect(outside.realpaths).toEqual([]);
  });

  it("on Windows goes behind a launcher only into a Git for Windows install, by the checks every PATH entry gets", () => {
    const repository = String.raw`C:\code\app`;
    const search = { platform: "win32" as const, home: String.raw`C:\Users\me`, cwd: repository };
    const layout = (root: string, ...more: string[]) => [String.raw`${root}\cmd\git.exe`, String.raw`${root}\mingw64\bin\git.exe`, ...more];
    const found = (path: string, files: readonly string[], links: Record<string, string> = {}) => findGit({ ...search, path }, {
      ...counted([String.raw`${repository}\.git`, ...files]),
      realpath: (folder: string) => Object.entries(links).reduce((real, [link, target]) => real.startsWith(link) ? target + real.slice(link.length) : real, folder),
    }).path;
    const installed = String.raw`C:\Program Files\Git`;
    expect(found(String.raw`${installed}\cmd`, layout(installed, String.raw`${installed}\etc\gitconfig`))).toBe(String.raw`${installed}\mingw64\bin\git.exe`);
    expect(found(String.raw`${installed}\cmd`, layout(installed, String.raw`${installed}\mingw64\share\git`))).toBe(String.raw`${installed}\mingw64\bin\git.exe`);
    // A launcher someone put on PATH beside a git.exe of their choosing, with nothing of Git for Windows around it.
    const planted = String.raw`D:\tools\planted`;
    expect(found(String.raw`${planted}\cmd`, layout(planted))).toBe(String.raw`${planted}\cmd\git.exe`);
    // A real install whose mingw64 folder is a junction into the repository Rerail runs in.
    const linked = String.raw`D:\portable\Git`;
    const junction = { [String.raw`${linked}\mingw64`]: String.raw`${repository}\vendor\mingw64` };
    expect(found(String.raw`${linked}\cmd`, layout(linked, String.raw`${linked}\etc\gitconfig`), junction)).toBe(String.raw`${linked}\cmd\git.exe`);
  });

  it("lists every place it looked, in gitCandidates' order, when no Git runs", () => {
    const search = { platform: "linux" as const, home: "/home/me", path: entries.join(":"), cwd: undefined };
    const found = findGit(search, counted([]));
    expect(found.path).toBeUndefined();
    expect(found.looked).toEqual(gitCandidates(search, counted([])));
    expect(new GitUnavailableError(found.looked).message).toContain("/opt/tool-39/bin/git, /usr/bin/git");
  });
});

describe("gitInstalled", () => {
  const probe = (files: readonly string[], developerDir?: string) => ({
    exists: (path: string) => files.includes(path),
    runnable: (path: string) => files.includes(path),
    developerDir: () => developerDir,
  });

  it("takes macOS's /usr/bin/git, always present as a stub, only with Apple's developer tools that carry Git", () => {
    // No Command Line Tools and no Xcode: the stub would open the installer, so Rerail says Git is missing.
    expect(gitInstalled("/usr/bin/git", "darwin", probe(["/usr/bin/git"]))).toBe(false);
    expect(gitInstalled("/usr/bin/git", "darwin", probe(["/usr/bin/git", "/Library/Developer/CommandLineTools/usr/bin/git"]))).toBe(true);
    const xcode = "/Applications/Xcode.app/Contents/Developer";
    expect(gitInstalled("/usr/bin/git", "darwin", probe(["/usr/bin/git", `${xcode}/usr/bin/git`], xcode))).toBe(true);
    expect(gitInstalled("/usr/bin/git", "darwin", probe(["/usr/bin/git"], xcode))).toBe(false);
  });

  it("takes any other Git that is there, Homebrew's on a Mac included", () => {
    expect(gitInstalled("/opt/homebrew/bin/git", "darwin", probe(["/opt/homebrew/bin/git"]))).toBe(true);
    expect(gitInstalled("/usr/bin/git", "linux", probe(["/usr/bin/git"]))).toBe(true);
    expect(gitInstalled("/usr/local/bin/git", "linux", probe([]))).toBe(false);
  });

  it.skipIf(process.platform === "win32")("passes over a folder named git and a git without its execute bit, as a shell does", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-git-runnable-"));
    roots.push(root);
    await mkdir(join(root, "folder", "git"), { recursive: true });
    await mkdir(join(root, "plain"));
    await writeFile(join(root, "plain", "git"), "#!/bin/sh\n");
    await mkdir(join(root, "runs"));
    await writeFile(join(root, "runs", "git"), "#!/bin/sh\n");
    await chmod(join(root, "runs", "git"), 0o755);

    expect(gitInstalled(join(root, "folder", "git"), "linux")).toBe(false);
    expect(gitInstalled(join(root, "plain", "git"), "linux")).toBe(false);
    expect(gitInstalled(join(root, "runs", "git"), "linux")).toBe(true);
  });
});
