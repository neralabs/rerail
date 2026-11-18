/**
 * Git for Windows' own git.exe, which Rerail starts without the launcher `<Git>\cmd\git.exe` that would start it, runs
 * what a repository or the person's configuration names (a filter, an alias) as it runs it through that launcher.
 * Windows only: each test starts this machine's Git for Windows, in a process environment the test sets, through
 * git.ts loaded afresh.
 */
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join, win32 } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import { removeFolder } from "../support/platform/remove-folder.js";

type GitModule = typeof import("../../src/engine/git.js");

const WINDOWS = process.platform === "win32";
const roots: string[] = [];

afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd, encoding: "utf8" });
}

/** git.ts as a new process loads it: it looks for Git with this process's environment as the test set it. */
async function freshGit(): Promise<GitModule> {
  vi.resetModules();
  return await import("../../src/engine/git.js");
}

async function repository(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-git-launcher-"));
  roots.push(root);
  git(root, "init", "--quiet");
  return root;
}

/** This machine's Git for Windows launcher, `<Git>\cmd\git.exe`: three folders above Git's `<Git>\<build>\libexec\git-core`. */
function machineLauncher(): string {
  const execPath = win32.normalize(execFileSync("git", ["--exec-path"], { encoding: "utf8" }).trim());
  return win32.join(win32.dirname(win32.dirname(win32.dirname(execPath))), "cmd", "git.exe");
}

/**
 * The `PATH` Windows gives a person's programs outside Git Bash, with Git for Windows installed as its installer
 * proposes: the system's folders, the launcher's `cmd` folder, and Node's. No `sh.exe` is on it.
 */
function personPath(launcher: string): string {
  const system = process.env.SystemRoot ?? String.raw`C:\Windows`;
  const folders = [join(system, "System32"), system, join(system, "System32", "Wbem"), win32.dirname(launcher), win32.dirname(process.execPath)];
  expect(folders.filter(folder => existsSync(join(folder, "sh.exe")))).toEqual([]);
  return folders.join(delimiter);
}

describe.runIf(WINDOWS)("Git for Windows' own git.exe started without its launcher", () => {
  it("lists a repository's changed files through a filter Git runs in its shell, with MSYSTEM set and no sh.exe on PATH", async () => {
    const root = await repository();
    await writeFile(join(root, ".gitattributes"), "*.txt filter=spaced\n");
    // A filter command with an argument runs through Git's shell, as Git LFS's and nbstripout's do.
    git(root, "config", "filter.spaced.clean", "cat -");
    await writeFile(join(root, "a.txt"), "a\n");
    await writeFile(join(root, "b.py"), "b = 1\n");
    git(root, "add", ".");
    git(root, "commit", "--quiet", "-m", "seed");
    // a.txt keeps its text under a newer time, so Git runs its filter to compare it with the index; b.py changed.
    const later = new Date(Date.now() + 120_000);
    await utimes(join(root, "a.txt"), later, later);
    await writeFile(join(root, "b.py"), "b = 2\n");
    // MSYSTEM as an MSYS2 set-up leaves it in the account's environment.
    vi.stubEnv("MSYSTEM", "MINGW64");
    vi.stubEnv("PATH", personPath(machineLauncher()));
    const started = await freshGit();
    await expect(started.gitModifiedOrUntrackedListing(root)).resolves.toEqual({ ended: "printed", paths: ["b.py"] });
    expect(started.runGit(root, ["diff", "--name-only"])).toMatchObject({ status: 0, stdout: "b.py\n" });
    expect(started.runGit(root, ["var", "GIT_SHELL_PATH"]).status).toBe(0);
  });

  it("starts Git so that a program it runs gets the environment it gets through Git for Windows' launcher", async () => {
    const root = await repository();
    const script = join(root, "environment.cjs");
    await writeFile(script, "require('node:fs').writeFileSync(process.argv[2], JSON.stringify(process.env));\n");
    // Git runs an alias command with no shell character in it without its shell: the program gets Git's environment.
    const node = process.execPath.replaceAll("\\", "/");
    git(root, "config", "alias.environment", /\s/u.test(node) ? `!"${node}"` : `!${node}`);
    const launcher = machineLauncher();
    const environment = async (file: string): Promise<Record<string, string>> => Object.fromEntries(
      Object.entries(JSON.parse(await readFile(file, "utf8")) as Record<string, string>).map(([name, value]) => [name.toUpperCase(), value]),
    );
    // HOMEDRIVE and HOMEPATH name the profile, as in a person's usual environment (the test's profile is a folder of its
    // own, while HOMEDRIVE and HOMEPATH still name the machine's account).
    const profile = process.env.USERPROFILE ?? root;
    const usual = { HOMEDRIVE: profile.slice(0, 2), HOMEPATH: profile.slice(2) };
    const variants: Record<string, string | undefined>[] = [
      { MSYSTEM: undefined, HOME: undefined, PLINK_PROTOCOL: undefined, ...usual },
      { MSYSTEM: "MINGW64", HOME: join(root, "home"), PLINK_PROTOCOL: undefined, ...usual },
      { MSYSTEM: "UCRT64", HOME: undefined, PLINK_PROTOCOL: "plink", ...usual },
    ];
    for (const [index, variant] of variants.entries()) {
      vi.unstubAllEnvs();
      for (const [name, value] of Object.entries(variant)) vi.stubEnv(name, value);
      vi.stubEnv("PATH", personPath(launcher));
      const started = await freshGit();
      expect(started.runGit(root, ["environment", script, join(root, `rerail-${index}.json`)]).status).toBe(0);
      // The same Git call through the launcher: Rerail's own settings and environment for every Git it starts.
      execFileSync(launcher, ["-c", "core.fsmonitor=false", "-c", "core.hooksPath=NUL", "-C", root, "environment", script, join(root, `launcher-${index}.json`)], {
        env: { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1" },
      });
      const [rerail, through] = await Promise.all([environment(join(root, `rerail-${index}.json`)), environment(join(root, `launcher-${index}.json`))]);
      const names = [...new Set([...Object.keys(rerail), ...Object.keys(through)])].sort();
      const differ = Object.fromEntries(names.filter(name => rerail[name] !== through[name]).map(name => [name, { rerail: rerail[name], launcher: through[name] }]));
      expect(differ, JSON.stringify(variant)).toEqual({});
    }
  });

  it("gives Git the launcher's HOME where HOMEDRIVE and HOMEPATH name a home share, there or gone, with both homes' bin on PATH", async () => {
    const root = await repository();
    const script = join(root, "environment.cjs");
    await writeFile(script, "require('node:fs').writeFileSync(process.argv[2], JSON.stringify(process.env));\n");
    const node = process.execPath.replaceAll("\\", "/");
    git(root, "config", "alias.environment", /\s/u.test(node) ? `!"${node}"` : `!${node}`);
    const launcher = machineLauncher();
    const profile = process.env.USERPROFILE ?? root;
    const read = async (file: string): Promise<Record<string, string>> => Object.fromEntries(
      Object.entries(JSON.parse(await readFile(file, "utf8")) as Record<string, string>).map(([name, value]) => [name.toUpperCase(), value]),
    );
    await mkdir(join(root, "share"));
    // The share there, then gone: the launcher, and git.exe, take it when that folder exists, else the profile.
    for (const [index, share] of [join(root, "share"), join(root, "gone")].entries()) {
      vi.unstubAllEnvs();
      for (const [name, value] of Object.entries({ HOME: undefined, HOMEDRIVE: share.slice(0, 2), HOMEPATH: share.slice(2), PATH: personPath(launcher) })) vi.stubEnv(name, value);
      const started = await freshGit();
      expect(started.runGit(root, ["environment", script, join(root, `rerail-${index}.json`)]).status).toBe(0);
      execFileSync(launcher, ["-c", "core.fsmonitor=false", "-c", "core.hooksPath=NUL", "-C", root, "environment", script, join(root, `launcher-${index}.json`)], {
        env: { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1" },
      });
      const [rerail, through] = await Promise.all([read(join(root, `rerail-${index}.json`)), read(join(root, `launcher-${index}.json`))]);
      expect(rerail.HOME, share).toBe(through.HOME);
      expect(through.HOME, share).toBe(index === 0 ? share : profile);
      // At a process's first start, before any answer about the share, Rerail's PATH is the launcher's with the other
      // home's bin beside the launcher's own, the share's first.
      const [shareBin, profileBin] = [win32.join(share, "bin"), win32.join(profile, "bin")];
      const launcherHomeBin = index === 0 ? shareBin : profileBin;
      expect(rerail.PATH, share).toBe(through.PATH?.replace(`${launcherHomeBin};`, `${shareBin};${profileBin};`));
    }
  });

  it("runs a program only in the profile's bin as the launcher does once the home share is known to be a folder: not at all", async () => {
    const root = await repository();
    const [share, profile] = [join(root, "share"), join(root, "profile")];
    await Promise.all([mkdir(join(share, "bin"), { recursive: true }), mkdir(join(profile, "bin"), { recursive: true })]);
    // An alias Git runs through its shell, which finds the program on Git's PATH.
    await writeFile(join(profile, "bin", "rerail-only-profile"), "#!/bin/sh\necho only-profile\n");
    git(root, "config", "alias.only", "!rerail-only-profile");
    const launcher = machineLauncher();
    for (const [name, value] of Object.entries({ HOME: undefined, HOMEDRIVE: share.slice(0, 2), HOMEPATH: share.slice(2), USERPROFILE: profile, PATH: personPath(launcher) })) vi.stubEnv(name, value);
    const started = await freshGit();
    // The first start has the share looked at off the thread; a call that does not block lets that look answer.
    expect((await started.runGitAsync(root, ["rev-parse", "--git-dir"])).status).toBe(0);
    const through = spawnSync(launcher, ["-c", "core.fsmonitor=false", "-c", "core.hooksPath=NUL", "-C", root, "only"], {
      encoding: "utf8", env: { ...process.env, GIT_OPTIONAL_LOCKS: "0", GIT_NO_LAZY_FETCH: "1" },
    });
    const rerail = started.runGit(root, ["only"]);
    // The launcher's HOME is the share, so its PATH holds the share's bin alone, and the program is not found.
    expect([rerail.status, rerail.stdout]).toEqual([through.status, through.stdout]);
    expect(through.status).not.toBe(0);
  });
});
