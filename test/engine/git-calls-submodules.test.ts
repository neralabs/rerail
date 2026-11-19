import * as childProcess from "node:child_process";
import { execFileSync, spawn } from "node:child_process";
import { chmodSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, renameSync, unlinkSync, writeFileSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import { runGit, type GitResult } from "../../src/engine/git.js";
import { removeFolder } from "../support/platform/remove-folder.js";

vi.mock("node:child_process", { spy: true });
const actualChildProcess = await vi.importActual<typeof import("node:child_process")>("node:child_process");

const roots: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/** The test's own Git, which looks into submodules and runs every driver their config names. */
function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "protocol.file.allow=always", ...args], {
    cwd, encoding: "utf8",
  });
}

async function folder(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-git-submodules-"));
  roots.push(root);
  return root;
}

async function repository(root: string, name: string, files: Record<string, string>): Promise<string> {
  const path = join(root, name);
  await mkdir(path);
  git(path, "init", "--quiet");
  git(path, "config", "core.autocrlf", "false");
  for (const [file, text] of Object.entries(files)) await writeFile(join(path, file), text);
  git(path, "add", ".");
  git(path, "commit", "--quiet", "-m", "seed");
  return path;
}

function recording(marker: string): string {
  return `echo ran >> '${marker.replaceAll("\\", "/")}'; cat`;
}

function runs(marker: string): number {
  return existsSync(marker) ? readFileSync(marker, "utf8").split("\n").filter(line => line !== "").length : 0;
}

const listed = (stdout: string): string[] => stdout.split("\0").filter(path => path !== "").sort();

/** The audit's `git diff` (`audit-control.ts`) and the scan's (`headless-scan.ts`), as each runs it. */
const AUDIT_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"];
const SCAN_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--diff-filter=ACMRTUXB", "-z", "HEAD", "--"];

describe("Rerail's diffs start no program a submodule's config names", () => {
  it("as Git starts no `git status` of its own in a submodule: Rerail asks each one itself", async () => {
    const root = await folder();
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    await writeFile(join(work, "sub", "inner.txt"), "two\n");
    const trace = join(root, "trace");
    await mkdir(trace);
    vi.stubEnv("GIT_TRACE2_EVENT", trace);

    const answers = [runGit(work, AUDIT_DIFF), runGit(work, SCAN_DIFF)];

    vi.stubEnv("GIT_TRACE2_EVENT", undefined);
    // Each Git process writes its events to a file of its own; Git notes each process it starts as `child_start`.
    const events = readdirSync(trace).flatMap(name => readFileSync(join(trace, name), "utf8").split("\n").filter(line => line !== ""));
    const started = events.map(line => JSON.parse(line) as { event: string; argv?: string[] }).filter(event => event.event === "child_start");
    expect(started.filter(event => event.argv?.includes("status"))).toEqual([]);
    expect(answers.map(answer => listed(answer.stdout))).toEqual([["sub"], ["sub"]]);
  }, 60_000);

  it("in a checked-out submodule whose own config names a filter program for a file rewritten at the same size", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const source = await repository(root, "source", { ".gitattributes": "*.txt filter=inner\n", "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    git(join(work, "sub"), "config", "filter.inner.clean", recording(marker));
    await writeFile(join(work, "sub", "inner.txt"), "two\n");

    const audit = runGit(work, AUDIT_DIFF);

    expect(runs(marker)).toBe(0);
    expect([audit.status, listed(audit.stdout)]).toEqual([0, ["sub"]]);
  }, 60_000);

  it("in a submodule outside safe.directory, and list it as Git does, which looks into it all the same", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const source = await repository(root, "source", { ".gitattributes": "*.txt filter=inner\n", "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    git(join(work, "sub"), "config", "filter.inner.clean", recording(marker));
    await writeFile(join(work, "sub", "inner.txt"), "two\n");
    // Every repository reads as another account's, and only the checkout is in safe.directory: Git still runs the
    // submodule's `git status` with GIT_DIR set, which skips the ownership check.
    const person = join(root, "person.gitconfig");
    await writeFile(person, `[safe]\n\tdirectory = ${realpathSync.native(work).replaceAll("\\", "/")}\n`);
    vi.stubEnv("GIT_CONFIG_GLOBAL", person);
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", "1");
    vi.stubEnv("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1");

    const answers = [runGit(work, AUDIT_DIFF), runGit(work, SCAN_DIFF)];

    expect(runs(marker)).toBe(0);
    expect(answers.map(answer => [answer.status, listed(answer.stdout)])).toEqual([[0, ["sub"]], [0, ["sub"]]]);
  }, 60_000);

  it("and answer as Git does when the environment names the checkout's index, as a Git hook's environment does", async () => {
    const root = await folder();
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    await writeFile(join(work, "top.txt"), "changed\n");
    // Git clears GIT_INDEX_FILE for the status it runs in a submodule, which then reads the submodule's own index.
    vi.stubEnv("GIT_INDEX_FILE", join(work, ".git", "index"));

    const answers = [runGit(work, AUDIT_DIFF), runGit(work, SCAN_DIFF)];

    expect(answers.map(answer => listed(answer.stdout))).toEqual([listed(git(work, ...AUDIT_DIFF)), listed(git(work, ...SCAN_DIFF))]);
    expect(answers.map(answer => listed(answer.stdout))).toEqual([["top.txt"], ["top.txt"]]);
  }, 60_000);

  it("and ask a checkout whose index records no submodule nothing more than that", async () => {
    const root = await folder();
    const work = await repository(root, "work", { "top.txt": "top\n" });
    await writeFile(join(work, "top.txt"), "changed\n");
    // Asked once first: each process asks Git's version once, at its first call that reads worktree files.
    runGit(work, AUDIT_DIFF);
    const trace = join(root, "trace");
    await mkdir(trace);
    vi.stubEnv("GIT_TRACE2_EVENT", trace);

    const audit = runGit(work, AUDIT_DIFF);

    vi.stubEnv("GIT_TRACE2_EVENT", undefined);
    // Each Git process writes its events to a file of its own: the config listing, the diff, and the index's submodules.
    expect(readdirSync(trace)).toHaveLength(3);
    expect(listed(audit.stdout)).toEqual(["top.txt"]);
  }, 60_000);

  it("and fail a diff where a submodule's `.git` file names no repository, as Git's own diff fails", async () => {
    const root = await folder();
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    // Git for Windows hides a submodule's `.git` file, and Windows refuses to open a hidden file for writing.
    unlinkSync(join(work, "sub", ".git"));
    await writeFile(join(work, "sub", ".git"), "gitdir: ../missing\n");

    const audit = runGit(work, AUDIT_DIFF);

    expect(() => git(work, ...AUDIT_DIFF)).toThrow();
    expect([audit.status === 0, audit.stdout]).toEqual([false, ""]);
  }, 60_000);

  it.skipIf(process.platform === "win32")("in the Git folder a submodule's `.git` named when Rerail looked, though it names a placed one by the status", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const { work, placed } = await submoduleWithPlacedCopy(root, marker);
    const gitfile = join(work, "sub", ".git");

    const audit = await afterSubmoduleListing(root, work, `printf 'gitdir: ../placed\\n' > '${gitfile}'`);

    expect(readFileSync(gitfile, "utf8")).toBe("gitdir: ../placed\n");
    expect(existsSync(placed)).toBe(true);
    expect(runs(marker)).toBe(0);
    expect([audit.status, listed(audit.stdout)]).toEqual([0, ["sub"]]);
  }, 60_000);

  it.skipIf(process.platform === "win32")("in a Git folder taken away while Rerail listed its config and put back after, and list that submodule as changed", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const { work, placed } = await submoduleWithPlacedCopy(root, marker);
    const kept = join(root, "kept");
    unlinkSync(join(work, "sub", ".git"));
    await writeFile(join(work, "sub", ".git"), "gitdir: ../placed\n");

    const audit = await afterSubmoduleListing(root, work, `mv '${kept}' '${placed}'`, `mv '${placed}' '${kept}'`);

    expect(existsSync(placed)).toBe(true);
    expect(runs(marker)).toBe(0);
    expect([audit.status, listed(audit.stdout)]).toEqual([0, ["sub"]]);
  }, 60_000);

  it("in a checked-out submodule or one inside it, and list them as Git does", async () => {
    const root = await folder();
    const marker = join(root, "driver-ran");
    const leaf = await repository(root, "leaf", { ".gitattributes": "*.txt filter=inner\n", "leaf.txt": "one\n" });
    const middle = await repository(root, "middle", { ".gitattributes": "*.txt filter=inner\n", "middle.txt": "one\n" });
    git(middle, "submodule", "add", "--quiet", leaf, "leaf");
    git(middle, "commit", "--quiet", "-m", "leaf");
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", middle, "sub");
    git(work, "submodule", "update", "--quiet", "--init", "--recursive");
    git(work, "commit", "--quiet", "-m", "submodule");
    git(join(work, "sub"), "config", "filter.inner.clean", recording(marker));
    git(join(work, "sub", "leaf"), "config", "filter.inner.clean", recording(marker));
    await writeFile(join(work, "sub", "leaf", "leaf.txt"), "two\n");

    const answers = [runGit(work, AUDIT_DIFF), runGit(work, SCAN_DIFF)];

    expect(runs(marker)).toBe(0);
    expect(answers.map(answer => [answer.status, listed(answer.stdout)])).toEqual([[0, ["sub"]], [0, ["sub"]]]);
  }, 60_000);

  it("in a Git folder a writer of the worktree alone placed for a registered submodule that was not checked out", async () => {
    const root = await folder();
    const marker = join(root, "driver-ran");
    const { work, gitfile } = await placedSubmodule(root, marker);
    await writeFile(gitfile, "gitdir: ../placed\n");

    const answers = [runGit(work, AUDIT_DIFF), runGit(work, SCAN_DIFF)];
    const rerailRuns = runs(marker);

    expect(rerailRuns).toBe(0);
    // Git's own diff, which runs the placed folder's driver, lists the submodule all the same.
    expect(answers.map(answer => listed(answer.stdout))).toEqual([listed(git(work, ...AUDIT_DIFF)), listed(git(work, ...SCAN_DIFF))]);
    expect(answers.map(answer => listed(answer.stdout))).toEqual([["sub"], ["sub"]]);
  }, 60_000);

  it("while that writer makes the Git folder appear and vanish as Rerail reads", async () => {
    const root = await folder();
    const marker = join(root, "driver-ran");
    const { work, gitfile } = await placedSubmodule(root, marker);
    const writer = spawn(process.execPath, ["-e", APPEAR_VANISH, gitfile, "gitdir: ../placed\n"], { stdio: "ignore" });
    try {
      for (let call = 0; call < 40; call += 1) runGit(work, AUDIT_DIFF);
    } finally {
      const ended = new Promise(settled => writer.once("exit", settled));
      writer.kill();
      await ended;
    }

    expect(runs(marker)).toBe(0);
  }, 120_000);
});

/**
 * A checkout `work` with a checked-out submodule `sub`, and `work/placed`: a copy of the submodule's Git folder, as a
 * writer of the worktree alone could place one, whose config names a driver for the submodule's changed `inner.txt`.
 */
async function submoduleWithPlacedCopy(root: string, marker: string): Promise<{ work: string; placed: string }> {
  const source = await repository(root, "source", { ".gitattributes": "*.txt filter=inner\n", "inner.txt": "one\n" });
  const work = await repository(root, "work", { "top.txt": "top\n" });
  git(work, "submodule", "add", "--quiet", source, "sub");
  git(work, "commit", "--quiet", "-m", "submodule");
  const placed = join(work, "placed");
  cpSync(join(work, ".git", "modules", "sub"), placed, { recursive: true });
  git(work, "config", "--file", join(placed, "config"), "--unset", "core.worktree");
  git(work, "config", "--file", join(placed, "config"), "filter.inner.clean", recording(marker));
  await writeFile(join(work, "sub", "inner.txt"), "two\n");
  return { work, placed };
}

/**
 * The audit's diff in `work`, run by a Rerail loaded with a Git that runs `command` right after the guard has listed
 * the config of the submodule `sub`, and `before` right before: what a writer of the worktree alone could do around
 * that listing.
 */
async function afterSubmoduleListing(root: string, work: string, command: string, before = ":"): Promise<{ status: number | null; stdout: string }> {
  const real = execFileSync("sh", ["-c", "command -v git"], { encoding: "utf8" }).trim();
  const bin = join(root, "bin");
  await mkdir(bin);
  const listing = `-C ${realpathSync.native(join(work, "sub"))} config -z --show-scope --list`;
  const around = (what: string): string => `case " $* " in *" ${listing} "*) ${what};; esac`;
  await writeFile(join(bin, "git"), ["#!/bin/sh", around(before), `'${real}' "$@"; code=$?`, around(command), "exit $code", ""].join("\n"));
  chmodSync(join(bin, "git"), 0o755);
  vi.stubEnv("PATH", `${bin}${delimiter}${process.env.PATH ?? ""}`);
  vi.resetModules();
  const fresh = await import("../../src/engine/git.js");
  return fresh.runGit(work, AUDIT_DIFF);
}

/** Makes `file` hold `text`, then removes it, as fast as it can. */
const APPEAR_VANISH = String.raw`
const fs = require("fs");
const [file, text] = process.argv.slice(1);
for (let n = 0; ; n++) {
  try {
    if (n & 1) fs.unlinkSync(file);
    else { fs.writeFileSync(file + ".t", text); fs.renameSync(file + ".t", file); }
  } catch {}
}`;

/**
 * A checkout `work` with a submodule `sub` that is registered and whose files are in place, but whose Git folder sits in
 * the worktree (`work/placed`) with a driver in its config for the changed `inner.txt`; `sub/.git` is absent, and a
 * writer that can write the worktree only makes it appear.
 */
async function placedSubmodule(root: string, marker: string): Promise<{ work: string; gitfile: string }> {
  const source = await repository(root, "source", { ".gitattributes": "*.txt filter=inner\n", "inner.txt": "one\n" });
  const work = await repository(root, "work", { "top.txt": "top\n" });
  git(work, "submodule", "add", "--quiet", source, "sub");
  git(work, "commit", "--quiet", "-m", "submodule");
  const placed = join(work, "placed");
  renameSync(join(work, ".git", "modules", "sub"), placed);
  git(work, "config", "--file", join(placed, "config"), "--unset", "core.worktree");
  git(work, "config", "--file", join(placed, "config"), "filter.inner.clean", recording(marker));
  const gitfile = join(work, "sub", ".git");
  unlinkSync(gitfile);
  await writeFile(join(work, "sub", "inner.txt"), "two\n");
  return { work, gitfile };
}

/**
 * A checkout with a submodule in each state a diff tells apart, `deep/moved` below a folder, and two holding a
 * submodule of their own, changed in its files or with untracked ones only.
 */
async function submodulesInEachState(root: string): Promise<string> {
  const lib = await repository(root, "lib", { "a.txt": "a\n", "b.txt": "b\n" });
  const middle = await repository(root, "middle", { "m.txt": "m\n" });
  git(middle, "submodule", "add", "--quiet", lib, "leaf");
  git(middle, "commit", "--quiet", "-m", "leaf");
  const work = await repository(root, "work", { "top.txt": "top\n" });
  for (const name of ["clean", "modified", "untracked", "staged", "deleted", "moved", "gone", "deep/moved"]) git(work, "submodule", "add", "--quiet", lib, name);
  for (const name of ["nested-modified", "nested-untracked"]) git(work, "submodule", "add", "--quiet", middle, name);
  git(work, "submodule", "update", "--quiet", "--init", "--recursive");
  git(work, "commit", "--quiet", "-m", "submodules");
  await writeFile(join(work, "modified", "a.txt"), "changed\n");
  await writeFile(join(work, "untracked", "new.txt"), "new\n");
  await writeFile(join(work, "staged", "a.txt"), "staged\n");
  git(join(work, "staged"), "add", "a.txt");
  await rm(join(work, "deleted", "b.txt"));
  for (const moved of ["moved", "deep/moved"]) git(join(work, moved), "commit", "--quiet", "--allow-empty", "-m", "moved");
  git(work, "submodule", "deinit", "--quiet", "--force", "gone");
  await writeFile(join(work, "nested-modified", "leaf", "a.txt"), "changed\n");
  await writeFile(join(work, "nested-untracked", "leaf", "new.txt"), "new\n");
  return work;
}

/** Each way the config tells a diff to treat submodules: how to set it in `work`, and how to undo it. */
const CONFIGS: ReadonlyArray<readonly [string, (work: string, root: string) => Promise<void> | void]> = [
  ["the defaults", () => undefined],
  [".gitmodules ignore = dirty", work => setGitmodules(work, "dirty")],
  [".gitmodules ignore = all", work => setGitmodules(work, "all")],
  [".gitmodules ignore = untracked", work => setGitmodules(work, "untracked")],
  ["the repository's submodule.<name>.ignore = none over .gitmodules' all", work => {
    setGitmodules(work, "all");
    for (const name of submoduleNames(work)) git(work, "config", `submodule.${name}.ignore`, "none");
  }],
  ["diff.ignoreSubmodules = all", work => void git(work, "config", "diff.ignoreSubmodules", "all")],
  ["status.showUntrackedFiles = no in the person's config", async (_work, root) => {
    const person = join(root, "person.gitconfig");
    await writeFile(person, "[status]\n\tshowUntrackedFiles = no\n");
    vi.stubEnv("GIT_CONFIG_GLOBAL", person);
  }],
  ["status.showUntrackedFiles = no in the submodules that hold one", work => {
    for (const middle of ["nested-modified", "nested-untracked"]) git(join(work, middle), "config", "status.showUntrackedFiles", "no");
  }],
  ["submodule.leaf.ignore = all in the submodules that hold one", work => {
    for (const middle of ["nested-modified", "nested-untracked"]) git(join(work, middle), "config", "submodule.leaf.ignore", "all");
  }],
];

function submoduleNames(work: string): string[] {
  return git(work, "config", "--file", ".gitmodules", "--get-regexp", "^submodule\\..*\\.path$").split("\n").filter(line => line !== "").map(line => line.split(" ")[1]!);
}

function setGitmodules(work: string, ignore: string): void {
  for (const name of submoduleNames(work)) git(work, "config", "--file", ".gitmodules", `submodule.${name}.ignore`, ignore);
}

describe("Rerail's diffs list the submodules Git's own diff lists", () => {
  it.each(CONFIGS)("with %s", async (_name, configure) => {
    const root = await folder();
    const work = await submodulesInEachState(root);
    await configure(work, root);

    const rerail = [runGit(work, AUDIT_DIFF), runGit(work, SCAN_DIFF), runGit(join(work, "deep"), AUDIT_DIFF)];
    const own = [git(work, ...AUDIT_DIFF), git(work, ...SCAN_DIFF), git(join(work, "deep"), ...AUDIT_DIFF)];

    expect(rerail.map(answer => answer.status)).toEqual([0, 0, 0]);
    expect(rerail.map(answer => answer.stdout)).toEqual(own);
  }, 120_000);

  it("in the defaults, which list a submodule with untracked files for the audit, and not for the scan", async () => {
    const root = await folder();
    const work = await submodulesInEachState(root);

    expect(listed(runGit(work, AUDIT_DIFF).stdout)).toEqual([
      "deep/moved", "deleted", "modified", "moved", "nested-modified", "nested-untracked", "staged", "untracked",
    ]);
    expect(listed(runGit(work, SCAN_DIFF).stdout)).toEqual(["deep/moved", "deleted", "modified", "moved", "nested-modified", "staged"]);
  }, 120_000);
});

/** A checkout `work` with a checked-out submodule `sub`: `top.txt` changed in the checkout, `inner.txt` in the submodule. */
async function changedSubmodule(root: string): Promise<string> {
  const source = await repository(root, "source", { "inner.txt": "one\n" });
  const work = await repository(root, "work", { "top.txt": "top\n" });
  git(work, "submodule", "add", "--quiet", source, "sub");
  git(work, "commit", "--quiet", "-m", "submodule");
  await writeFile(join(work, "top.txt"), "changed\n");
  await writeFile(join(work, "sub", "inner.txt"), "two\n");
  return work;
}

/** Whether Git's own `args` in `cwd` answered, and its stdout: the one Git call Rerail made before its guard. */
function own(cwd: string, args: readonly string[]): [boolean, string] {
  try {
    return [true, git(cwd, ...args)];
  } catch {
    return [false, ""];
  }
}

/** Whether Rerail's call `args` in `cwd` answered, and its stdout. */
function rerail(cwd: string, args: readonly string[]): [boolean, string] {
  const answer = runGit(cwd, args);
  return [answer.status === 0 && answer.error === undefined, answer.stdout];
}

/** The submodule `sub` of a `changedSubmodule` checkout with its Git folder made otherwise; the checkout to diff in. */
const GIT_FOLDERS: ReadonlyArray<readonly [string, (work: string, root: string) => Promise<string>]> = [
  ["whose config file is empty", async work => {
    writeFileSync(join(work, ".git", "modules", "sub", "config"), "");
    return work;
  }],
  ["with no config file", async work => {
    unlinkSync(join(work, ".git", "modules", "sub", "config"));
    return work;
  }],
  ["that holds no repository: an empty `.git` folder in a submodule not checked out", async work => {
    git(work, "submodule", "deinit", "--quiet", "--force", "sub");
    mkdirSync(join(work, "sub", ".git"));
    return work;
  }],
  ["in a linked worktree of a bare repository, its common directory", async (work, root) => {
    const commit = git(join(work, "sub"), "rev-parse", "HEAD").trim();
    git(work, "submodule", "deinit", "--quiet", "--force", "sub");
    await removeFolder(join(work, "sub"));
    git(root, "clone", "--quiet", "--bare", join(root, "source"), "bare.git");
    git(join(root, "bare.git"), "worktree", "add", "--quiet", "--detach", join(work, "sub"), commit);
    await writeFile(join(work, "sub", "inner.txt"), "two\n");
    return work;
  }],
  ["in a linked worktree of the submodule's own repository", async work => {
    const commit = git(join(work, "sub"), "rev-parse", "HEAD").trim();
    const gitFolder = join(work, ".git", "modules", "sub");
    await removeFolder(join(work, "sub"));
    git(work, "config", "--file", join(gitFolder, "config"), "--unset", "core.worktree");
    git(work, "--git-dir", gitFolder, "worktree", "add", "--quiet", "--detach", join(work, "sub"), commit);
    await writeFile(join(work, "sub", "inner.txt"), "two\n");
    return work;
  }],
  ["in a linked worktree of the checkout", async (work, root) => {
    const linked = join(root, "linked");
    git(work, "worktree", "add", "--quiet", "--detach", linked);
    git(linked, "submodule", "update", "--quiet", "--init");
    await writeFile(join(linked, "top.txt"), "changed\n");
    await writeFile(join(linked, "sub", "inner.txt"), "two\n");
    return linked;
  }],
];

describe("Rerail's diffs answer as Git's own for each Git folder a submodule's `.git` names", () => {
  it.each(GIT_FOLDERS)("one %s", async (_name, made) => {
    const root = await folder();
    const work = await made(await changedSubmodule(root), root);

    expect([rerail(work, AUDIT_DIFF), rerail(work, SCAN_DIFF)]).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
  }, 60_000);

  it.each(GIT_FOLDERS.slice(0, 2))("and list the submodule and the checkout's files for one %s, which defines no driver", async (_name, made) => {
    const root = await folder();
    const work = await made(await changedSubmodule(root), root);

    const answers = [runGit(work, AUDIT_DIFF), runGit(work, SCAN_DIFF)];

    expect(answers.map(answer => [answer.status, listed(answer.stdout)])).toEqual([[0, ["sub", "top.txt"]], [0, ["sub", "top.txt"]]]);
  }, 60_000);

  it("one of a submodule inside a submodule, whose config file is empty", async () => {
    const root = await folder();
    const leaf = await repository(root, "leaf", { "leaf.txt": "one\n" });
    const middle = await repository(root, "middle", { "middle.txt": "one\n" });
    git(middle, "submodule", "add", "--quiet", leaf, "leaf");
    git(middle, "commit", "--quiet", "-m", "leaf");
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", middle, "sub");
    git(work, "submodule", "update", "--quiet", "--init", "--recursive");
    git(work, "commit", "--quiet", "-m", "submodule");
    writeFileSync(join(work, ".git", "modules", "sub", "modules", "leaf", "config"), "");
    await writeFile(join(work, "sub", "leaf", "leaf.txt"), "two\n");

    const answers = [rerail(work, AUDIT_DIFF), rerail(work, SCAN_DIFF)];

    expect(answers).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
    expect(answers).toEqual([[true, "sub\0"], [true, "sub\0"]]);
  }, 60_000);
});

describe("Rerail's diff of a checkout with submodules keeps the call's one time limit", () => {
  it("as each Git it starts gets what is left of it, and none starts once it is spent", async () => {
    const root = await folder();
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    for (const name of ["one", "two", "three"]) git(work, "submodule", "add", "--quiet", source, name);
    git(work, "commit", "--quiet", "-m", "submodules");
    for (const name of ["one", "two", "three"]) await writeFile(join(work, name, "inner.txt"), "two\n");
    // Asked once first: each process asks Git's version once, at its first call that reads worktree files.
    runGit(work, AUDIT_DIFF);
    const now = performance.now.bind(performance);
    let used = 0;
    const clock = vi.spyOn(performance, "now").mockImplementation(() => now() + used);
    const limits: Array<number | undefined> = [];
    // Each Git answers as itself, and as if it had taken 4 s of the call's 10 s.
    vi.mocked(childProcess.spawnSync).mockImplementation(((file: string, args: readonly string[], options: childProcess.SpawnSyncOptions) => {
      limits.push(options.timeout);
      const result = actualChildProcess.spawnSync(file, args, options);
      used += 4_000;
      return result;
    }) as unknown as typeof childProcess.spawnSync);
    const answer = ((): GitResult => {
      try {
        return runGit(work, AUDIT_DIFF, { timeout: 10_000 });
      } finally {
        vi.mocked(childProcess.spawnSync).mockImplementation(actualChildProcess.spawnSync);
        clock.mockRestore();
      }
    })();

    // The config listing, the diff and the index's submodules ran; the question after them was not started.
    expect(limits.map((limit, at) => limit !== undefined && limit > 0 && limit <= 10_000 - 4_000 * at)).toEqual([true, true, true]);
    expect([answer.status, answer.stdout, answer.error?.message]).toEqual([null, "", expect.stringMatching(/^spawnSync .+ ETIMEDOUT$/u)]);
    expect(answer.error).toMatchObject({ code: "ETIMEDOUT", syscall: expect.stringMatching(/^spawnSync /u) });
  }, 60_000);
});
