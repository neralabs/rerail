/**
 * Git for Windows' own git.exe, which Rerail starts without the launcher `<Git>\cmd\git.exe` that would start it, runs
 * what a repository or the person's configuration names (a filter, an alias) as it runs it through that launcher.
 * Windows only: each test starts this machine's Git for Windows, in a process environment the test sets, through
 * git.ts loaded afresh.
 */
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, utimes, writeFile } from "node:fs/promises";
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
    const variants: Record<string, string | undefined>[] = [
      { MSYSTEM: undefined, HOME: undefined, PLINK_PROTOCOL: undefined },
      { MSYSTEM: "MINGW64", HOME: join(root, "home"), PLINK_PROTOCOL: undefined },
      { MSYSTEM: "UCRT64", HOME: undefined, PLINK_PROTOCOL: "plink" },
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
});
