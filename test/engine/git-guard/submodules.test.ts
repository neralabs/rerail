import * as childProcess from "node:child_process";
import { execFileSync, spawn } from "node:child_process";
import {
  appendFileSync, chmodSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, renameSync, symlinkSync, unlinkSync, writeFileSync,
} from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { availableParallelism, tmpdir } from "node:os";
import { delimiter, join, sep } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import { gitModifiedOrUntrackedPaths, runGit, runGitWithoutBlocking, type GitResult } from "../../../src/engine/git.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { unopenable, type Unopenable } from "../../support/platform/unopenable-file.js";

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

/** Whether the test's own Git answers `git <args>` in `cwd`, and its stdout ("" when it fails). */
function gitAnswer(cwd: string, args: readonly string[]): [boolean, string] {
  try {
    return [true, git(cwd, ...args)];
  } catch {
    return [false, ""];
  }
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

/** `work` with a filter driver in its own config, so the guard lists its config before each call that reads its files. */
function withDriver(work: string): string {
  git(work, "config", "filter.probe.clean", "cat");
  return work;
}

function recording(marker: string): string {
  return `echo ran >> '${marker.replaceAll("\\", "/")}'; cat`;
}

function runs(marker: string): number {
  return existsSync(marker) ? readFileSync(marker, "utf8").split("\n").filter(line => line !== "").length : 0;
}

const listed = (stdout: string): string[] => stdout.split("\0").filter(path => path !== "").sort();

/** What `call` answered, and the Git processes it started: each writes its trace2 events to a file of its own. */
async function gitsStarted<T>(root: string, call: () => T | Promise<T>): Promise<{ answer: T; started: number }> {
  const trace = await mkdtemp(join(root, "trace-"));
  vi.stubEnv("GIT_TRACE2_EVENT", trace);
  try {
    const answer = await call();
    return { answer, started: readdirSync(trace).length };
  } finally {
    vi.stubEnv("GIT_TRACE2_EVENT", undefined);
  }
}

/** The audit's `git diff` (`audit-control.ts`) and the scan's (`headless-scan.ts`), as each runs it. */
const AUDIT_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"];
const SCAN_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--diff-filter=ACMRTUXB", "-z", "HEAD", "--"];

/** A state made as asked, with nothing to undo. */
const MADE: Unopenable = { held: true, giveBack: () => undefined };

/**
 * Each state a marked submodule `sub` of a {@link submoduleGitfile} checkout is asked in, whose worktree changed: how
 * to make it from the checkout `work` and the line that names the Git folder, whether it holds, and how to undo it.
 */
const MARKED_STATES: ReadonlyArray<readonly [string, (work: string, named: string) => Unopenable]> = [
  ["whose worktree changed", () => MADE],
  ["whose `.git` file cannot be opened", work => unopenable(join(work, "sub", ".git"))],
  ["whose `.git` file holds more than 1 MiB", (work, named) => {
    writeFileSync(join(work, "sub", ".git"), named.padEnd(1024 * 1024 + 1, "\n"));
    return MADE;
  }],
  ["whose `.git` file holds no `gitdir: `", work => {
    writeFileSync(join(work, "sub", ".git"), "nothing\n");
    return MADE;
  }],
  ["in a checkout whose `.gitmodules` Git cannot read", work => {
    writeFileSync(join(work, ".gitmodules"), "[submodule\n");
    return MADE;
  }],
  // The scan's diff names it, as its commit changed, and its rule leaves it out of Git's own.
  ["whose commit changed in the index, with `ignore = all` in `.gitmodules`", work => {
    git(join(work, "sub"), "commit", "--quiet", "--allow-empty", "-m", "moved");
    git(work, "add", "sub");
    setGitmodules(work, "all");
    return MADE;
  }],
];

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

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

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

    const audit = await runGitWithoutBlocking(work, AUDIT_DIFF);

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

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

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

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

    expect(answers.map(answer => listed(answer.stdout))).toEqual([listed(git(work, ...AUDIT_DIFF)), listed(git(work, ...SCAN_DIFF))]);
    expect(answers.map(answer => listed(answer.stdout))).toEqual([["top.txt"], ["top.txt"]]);
  }, 60_000);

  it("and ask a checkout whose index records no submodule nothing more than that, from its top or a folder in it", async () => {
    const root = await folder();
    const work = await repository(root, "work", { "top.txt": "top\n", "notes.txt": "notes\n" });
    await mkdir(join(work, "src"));
    await writeFile(join(work, "src", "code.txt"), "code\n");
    git(work, "add", ".");
    git(work, "commit", "--quiet", "-m", "src");
    await writeFile(join(work, "top.txt"), "changed\n");
    await writeFile(join(work, "src", "code.txt"), "changed\n");
    // Asked once first: each process asks Git's version once, at its first call that reads worktree files.
    await runGitWithoutBlocking(work, AUDIT_DIFF);
    const traces = ["audit", "scan", "folder"].map(name => join(root, name));
    const calls: Array<readonly [string, readonly string[]]> = [[work, AUDIT_DIFF], [work, SCAN_DIFF], [join(work, "src"), AUDIT_DIFF]];

    const answers: GitResult[] = [];
    for (const [at, [cwd, args]] of calls.entries()) {
      await mkdir(traces[at]!);
      vi.stubEnv("GIT_TRACE2_EVENT", traces[at]!);
      answers.push(await runGitWithoutBlocking(cwd, args));
    }

    vi.stubEnv("GIT_TRACE2_EVENT", undefined);
    // Each Git process writes its events to a file of its own: the diff alone, as before the guard. Rerail reads the
    // config files and the index file in process, and starts no Git to list the config or the index's submodules.
    expect(traces.map(trace => readdirSync(trace).length)).toEqual([1, 1, 1]);
    expect(answers.map(answer => answer.stdout)).toEqual(calls.map(([cwd, args]) => git(cwd, ...args)));
  }, 60_000);

  it.each([
    ["defines no driver and has no worktree config: the call alone, as before the guard", () => undefined, [1, 1]],
    ["is past what Rerail reads of a config file: its listing, then the call", (gitFolder: string) => {
      appendFileSync(join(gitFolder, "config"), `# ${"x".repeat(64 * 1024)}\n`);
    }, [2, 2]],
  ] as const)("and start for a blocking call and the walk's listing, where the checkout's config %s", async (_name, change, gits) => {
    const root = await folder();
    const work = await repository(root, "work", { "top.txt": "top\n" });
    change(join(work, ".git"));
    await writeFile(join(work, "top.txt"), "changed\n");
    // Asked once first: each process asks Git's version once, at its first call that reads worktree files.
    runGit(work, ["ls-files", "-z", "--modified"]);

    const blocking = await gitsStarted(root, () => runGit(work, ["ls-files", "-z", "--modified"]));
    const walk = await gitsStarted(root, async () => await gitModifiedOrUntrackedPaths(work));

    expect([blocking.started, walk.started]).toEqual(gits);
    expect([blocking.answer, walk.answer]).toEqual([{ ...blocking.answer, status: 0, stdout: "top.txt\0" }, ["top.txt"]]);
  }, 60_000);

  // Windows has no pipe in a folder. Git waits on each pipe to the call's time limit, and Rerail's reads before it wait on none.
  it.skipIf(process.platform === "win32").each(["HEAD", "commondir"])("and end at the call's time limit, holding no thread, where the checkout's Git folder holds a pipe as its %s", async name => {
    const root = await folder();
    const work = await repository(root, "work", { "top.txt": "top\n" });
    runGit(work, ["ls-files", "-z", "--modified"]);
    const placed = join(work, ".git", name);
    await rm(placed, { force: true });
    execFileSync("mkfifo", [placed]);

    const answers = [runGit(work, AUDIT_DIFF, { timeout: 2_000 }), await runGitWithoutBlocking(work, AUDIT_DIFF, { timeout: 2_000 })];

    expect(answers.map(answer => [answer.status, (answer.error as NodeJS.ErrnoException | undefined)?.code])).toEqual([[null, "ETIMEDOUT"], [null, "ETIMEDOUT"]]);
  }, 60_000);

  it.skipIf(process.platform === "win32")("and end at the call's time limit, holding no thread, where a submodule's index is a pipe", async () => {
    const root = await folder();
    const work = await changedSubmodule(root);
    const index = join(work, ".git", "modules", "sub", "index");
    await rm(index);
    execFileSync("mkfifo", [index]);

    const audit = await runGitWithoutBlocking(work, AUDIT_DIFF, { timeout: 2_000 });

    expect([audit.status, (audit.error as NodeJS.ErrnoException | undefined)?.code]).toEqual([null, "ETIMEDOUT"]);
  }, 60_000);

  it.skipIf(process.platform === "win32")("and list a checkout's config whose worktree config is a pipe, which Git does not open here, holding no thread on it", async () => {
    const root = await folder();
    const work = await repository(root, "work", { "top.txt": "top\n" });
    execFileSync("mkfifo", [join(work, ".git", "config.worktree")]);
    await writeFile(join(work, "top.txt"), "changed\n");
    runGit(work, ["ls-files", "-z", "--modified"]);

    const audit = await gitsStarted(root, async () => await runGitWithoutBlocking(work, AUDIT_DIFF));

    expect([audit.started, audit.answer.stdout]).toEqual([2, git(work, ...AUDIT_DIFF)]);
  }, 60_000);

  it("and read the index of a linked worktree, not its main worktree's, to tell whether it records a submodule", async () => {
    const root = await folder();
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    const linked = join(root, "linked");
    git(work, "worktree", "add", "--quiet", "-b", "with-submodule", linked);
    git(linked, "submodule", "add", "--quiet", source, "sub");
    git(linked, "commit", "--quiet", "-m", "submodule");
    await writeFile(join(linked, "sub", "inner.txt"), "two\n");

    const answers = [await runGitWithoutBlocking(linked, AUDIT_DIFF), await runGitWithoutBlocking(linked, SCAN_DIFF)];

    expect(answers.map(answer => answer.stdout)).toEqual([git(linked, ...AUDIT_DIFF), git(linked, ...SCAN_DIFF)]);
    expect(listed(answers[0]!.stdout)).toEqual(["sub"]);
  }, 60_000);

  it("and add no submodule to a diff of two commits, of a range or of a merge base, which reads no worktree", async () => {
    const root = await folder();
    const work = await changedSubmodule(root);
    git(work, "commit", "--quiet", "-m", "top", "top.txt");
    const forms = [["HEAD~1", "HEAD"], ["HEAD~1..HEAD"], ["HEAD~1...HEAD"], ["HEAD^!"], ["--merge-base", "HEAD~1", "HEAD"], ["HEAD~1"]];
    const diffs = forms.map(revisions => ["diff", "--name-only", "-z", ...revisions, "--"]);

    const answers = await Promise.all(diffs.map(async args => await runGitWithoutBlocking(work, args)));

    expect(answers.map(answer => answer.stdout)).toEqual(diffs.map(args => git(work, ...args)));
    // Only the diff of one commit against the worktree lists the submodule whose worktree changed.
    expect(answers.map(answer => listed(answer.stdout))).toEqual([...forms.slice(0, -1).map(() => ["top.txt"]), ["sub", "top.txt"]]);
  }, 60_000);

  it("and add no submodule to a diff whose filter leaves changed entries out, as Git's own adds none", async () => {
    const root = await folder();
    const work = await changedSubmodule(root);
    const diffs = ["--diff-filter=D", "--diff-filter=m"].map(filter => ["diff", "--name-only", "-z", filter, "--"]);

    const answers = await Promise.all(diffs.map(async args => await runGitWithoutBlocking(work, args)));

    expect(answers.map(answer => [answer.status, answer.stdout])).toEqual(diffs.map(args => [0, git(work, ...args)]));
    expect(answers.map(answer => answer.stdout)).toEqual(["", ""]);
  }, 60_000);

  it("and list a submodule whose status prints past what Rerail reads of it, as Git lists it", async () => {
    const root = await folder();
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    // About 330 KB of status entries, past the 64 KiB Rerail reads of a submodule's status and the pipe's buffer.
    for (let at = 0; at < 3000; at += 1) writeFileSync(join(work, "sub", `untracked-${String(at).padStart(4, "0")}-${"x".repeat(90)}.txt`), "");
    await runGitWithoutBlocking(work, SCAN_DIFF);
    const trace = join(root, "trace");
    await mkdir(trace);
    vi.stubEnv("GIT_TRACE2_EVENT", trace);

    const audit = await runGitWithoutBlocking(work, AUDIT_DIFF);

    vi.stubEnv("GIT_TRACE2_EVENT", undefined);
    const scan = await runGitWithoutBlocking(work, SCAN_DIFF);
    expect([audit, scan].map(answer => [answer.status, answer.stdout])).toEqual([[0, git(work, ...AUDIT_DIFF)], [0, git(work, ...SCAN_DIFF)]]);
    expect([audit, scan].map(answer => listed(answer.stdout))).toEqual([["sub"], []]);
    // Rerail ended the submodule's status once it had read past that: no event of its Git says it exited with 0.
    const statuses = readdirSync(trace).map(name => readFileSync(join(trace, name), "utf8")).filter(events => events.includes("\"status\",\"--porcelain=2\""));
    expect([statuses.length, statuses.some(events => /"event":"exit".*"code":0[,}]/u.test(events))]).toEqual([1, false]);
  }, 60_000);

  // Windows has no folder its owner cannot open, and the superuser opens every folder.
  it.skipIf(process.platform === "win32" || process.getuid?.() === 0)("and leave out a submodule whose status lists nothing and warns past what Rerail reads of its output, as Git leaves it out", async () => {
    const root = await folder();
    const { work, shut } = await warningSubmodule(root);
    try {
      const answers = [await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)];

      expect(answers).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
      expect(answers).toEqual([[true, ""], [true, ""]]);
    } finally {
      for (const path of shut) chmodSync(path, 0o755);
    }
  }, 60_000);

  it.skipIf(process.platform === "win32" || process.getuid?.() === 0)("and fail a diff where a submodule's status warns past the most the guard reads, as a question past its cap fails", async () => {
    const root = await folder();
    const { work, shut } = await warningSubmodule(root);
    // The guard's cap at 32 KiB, Node's longest string (96 KiB here) less 64 KiB: past the config listings, under the
    // submodule's 90 KB of warnings.
    vi.doMock("node:buffer", async original => {
      const actual = await original<typeof import("node:buffer")>();
      return { ...actual, constants: { ...actual.constants, MAX_STRING_LENGTH: 96 * 1024 } };
    });
    vi.resetModules();
    try {
      const fresh = await import("../../../src/engine/git.js");

      const audit = await fresh.runGitWithoutBlocking(work, AUDIT_DIFF);

      const error = audit.error as (NodeJS.ErrnoException & { stream?: string }) | undefined;
      expect([audit.status, audit.stdout, error?.code, error?.stream]).toEqual([null, "", "ENOBUFS", "stderr"]);
    } finally {
      vi.doUnmock("node:buffer");
      for (const path of shut) chmodSync(path, 0o755);
    }
  }, 60_000);

  it("and ask Git which index it reads where the diff runs in a Git folder whose core.worktree names the checkout", async () => {
    const root = await folder();
    // The folder above holds a repository of its own, whose index records no submodule.
    git(root, "init", "--quiet");
    await writeFile(join(root, "outer.txt"), "outer\n");
    git(root, "add", "outer.txt");
    git(root, "commit", "--quiet", "-m", "outer");
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    await writeFile(join(work, "sub", "inner.txt"), "two\n");
    const store = join(root, "store.git");
    renameSync(join(work, ".git"), store);
    git(store, "config", "core.worktree", work);
    git(store, "config", "--file", join(store, "modules", "sub", "config"), "core.worktree", join(work, "sub"));
    // Git for Windows hides a submodule's `.git` file, and Windows refuses to open a hidden file for writing.
    unlinkSync(join(work, "sub", ".git"));
    await writeFile(join(work, "sub", ".git"), `gitdir: ${join(store, "modules", "sub").replaceAll("\\", "/")}\n`);

    const answers = [await runGitWithoutBlocking(store, AUDIT_DIFF), await runGitWithoutBlocking(store, SCAN_DIFF)];

    expect(answers.map(answer => answer.stdout)).toEqual([git(store, ...AUDIT_DIFF), git(store, ...SCAN_DIFF)]);
    expect(listed(answers[0]!.stdout)).toEqual(["sub"]);
  }, 60_000);

  it("and read the index the environment names, as a Git hook's environment names one, to tell whether it records a submodule", async () => {
    const root = await folder();
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    const index = join(root, "other-index");
    cpSync(join(work, ".git", "index"), index);
    vi.stubEnv("GIT_INDEX_FILE", index);
    git(work, "submodule", "add", "--quiet", source, "sub");
    await writeFile(join(work, "sub", "inner.txt"), "two\n");

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

    expect(answers.map(answer => answer.stdout)).toEqual([git(work, ...AUDIT_DIFF), git(work, ...SCAN_DIFF)]);
    expect(answers.map(answer => listed(answer.stdout))).toEqual([["sub"], [".gitmodules", "sub"]]);
  }, 60_000);

  it("and ask a clean checked-out submodule whose index records no submodule only its config and its status", async () => {
    const root = await folder();
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    await writeFile(join(work, "top.txt"), "changed\n");
    await runGitWithoutBlocking(work, AUDIT_DIFF);
    const trace = join(root, "trace");
    await mkdir(trace);
    vi.stubEnv("GIT_TRACE2_EVENT", trace);

    const audit = await runGitWithoutBlocking(work, AUDIT_DIFF);

    vi.stubEnv("GIT_TRACE2_EVENT", undefined);
    // The checkout's diff, its index's submodules, its top folder and its config listing (the audit's diff names
    // `--ignore-submodules=none`, so `.gitmodules` is not read); then the submodule's status alone, as Git's own diff
    // starts it: Rerail reads its config and its index in process, and lists neither.
    expect(readdirSync(trace)).toHaveLength(5);
    expect(audit.stdout).toBe(git(work, ...AUDIT_DIFF));
  }, 60_000);

  it("and ask the submodules inside a submodule whose split index keeps them in its shared file", async () => {
    const root = await folder();
    const leaf = await repository(root, "leaf", { "leaf.txt": "one\n" });
    const middle = await repository(root, "middle", { "middle.txt": "one\n" });
    git(middle, "submodule", "add", "--quiet", leaf, "leaf");
    git(middle, "commit", "--quiet", "-m", "leaf");
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", middle, "sub");
    git(work, "submodule", "update", "--quiet", "--init", "--recursive");
    git(work, "commit", "--quiet", "-m", "submodule");
    // Every entry moves to the shared file, and the index itself holds only the `link` extension that names it.
    git(join(work, "sub"), "update-index", "--split-index");
    await writeFile(join(work, "sub", "leaf", "leaf.txt"), "changed\n");

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

    expect(answers.map(answer => listed(answer.stdout))).toEqual([["sub"], ["sub"]]);
    expect(answers.map(answer => answer.stdout)).toEqual([git(work, ...AUDIT_DIFF), git(work, ...SCAN_DIFF)]);
  }, 60_000);

  it("and start one Git for a call that reads no worktree file, and fail one that prints past its limit as Git's start does", async () => {
    const root = await folder();
    const work = await repository(root, "work", { "top.txt": "top\n" });
    await runGitWithoutBlocking(work, AUDIT_DIFF);
    const trace = join(root, "trace");
    await mkdir(trace);
    vi.stubEnv("GIT_TRACE2_EVENT", trace);

    const cached = await runGitWithoutBlocking(work, ["diff", "--cached", "--name-only", "-z"]);

    vi.stubEnv("GIT_TRACE2_EVENT", undefined);
    const printed = await runGitWithoutBlocking(work, ["ls-files", "-z"], { maxBuffer: 1 });
    expect([cached.status, readdirSync(trace)]).toEqual([0, [expect.any(String)]]);
    expect([printed.status, printed.error?.message]).toEqual([null, "Git printed more than maxBuffer (1 bytes) to stdout"]);
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

    const audit = await runGitWithoutBlocking(work, AUDIT_DIFF);

    expect(() => git(work, ...AUDIT_DIFF)).toThrow();
    expect([audit.status === 0, audit.stdout]).toEqual([false, ""]);
    // The path as Git joins it from the submodule's folder, so a diff with many submodules names the one that failed.
    expect(audit.stderr).toMatch(/^fatal: not a git repository: .+[\\/]sub\/\.\.\/missing\n$/u);
  }, 60_000);

  it.skipIf(process.platform === "win32")("and take a submodule whose `.git` is a pipe for one not checked out, as Git's own diff takes it", async () => {
    const root = await folder();
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    unlinkSync(join(work, "sub", ".git"));
    execFileSync("mkfifo", [join(work, "sub", ".git")]);
    await writeFile(join(work, "top.txt"), "changed\n");

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

    expect(answers.map(answer => [answer.status, answer.stdout])).toEqual([[0, git(work, ...AUDIT_DIFF)], [0, git(work, ...SCAN_DIFF)]]);
  }, 60_000);

  it("and read a submodule's `.git` file of 100 KiB as Git reads it", async () => {
    const root = await folder();
    const { work, gitfile, named } = await submoduleGitfile(root);
    // Git takes off every line end after the path.
    await writeFile(gitfile, named.padEnd(100 * 1024, "\n"));
    await writeFile(join(work, "sub", "inner.txt"), "two\n");

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

    expect(answers.map(answer => [answer.status === 0, answer.stdout])).toEqual([gitAnswer(work, AUDIT_DIFF), gitAnswer(work, SCAN_DIFF)]);
    expect(answers.map(answer => answer.status === 0)).toEqual([true, true]);
  }, 60_000);

  // Each row: what the `.git` file is, its text from the line that named the Git folder, and Git's message, `%s` its path.
  it.each<[string, (named: string) => string, string]>([
    ["holds more than 1 MiB", named => named.padEnd(1024 * 1024 + 1, "\n"), String.raw`too large to be a \.git file: '%s'`],
    ["holds no `gitdir: `", () => "nothing\n", "invalid gitfile format: %s"],
    ["holds no path", () => "gitdir: \n", "no path in gitfile: %s"],
  ])("and fail a diff where a submodule's `.git` file %s, with Git's message, as Git's own diff fails", async (_what, text, said) => {
    const root = await folder();
    const { work, gitfile, named } = await submoduleGitfile(root);
    await writeFile(gitfile, text(named));

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];
    const gits = [gitStderr(work, AUDIT_DIFF), gitStderr(work, SCAN_DIFF)];

    const message = new RegExp(`^fatal: ${said.replace("%s", String.raw`(?:.+[\\/])?sub[\\/]\.git`)}\n$`, "u");
    expect([...gits, ...answers.map(answer => answer.stderr)]).toEqual(Array.from({ length: 4 }, () => expect.stringMatching(message)));
    expect(answers.map(answer => [answer.status === 0, answer.stdout])).toEqual([[false, ""], [false, ""]]);
  }, 60_000);

  // Git's diff opens a submodule's `.git` to find its commit before it asks the submodule, so Rerail's own diff fails
  // there first, with Git's message.
  it("and answer as Git's own diff where a submodule's `.git` file cannot be opened", async context => {
    const root = await folder();
    const { work, gitfile, named } = await submoduleGitfile(root);
    await writeFile(gitfile, `${named}\n`);
    const denied = unopenable(gitfile);
    try {
      if (!denied.held) context.skip();
      const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];
      const gits = [gitStderr(work, AUDIT_DIFF), gitStderr(work, SCAN_DIFF)];

      expect(answers.map(answer => answer.status === 0 ? undefined : answer.stderr)).toEqual(gits);
      expect(gits.map(stderr => stderr === undefined)).toEqual([false, false]);
    } finally {
      denied.giveBack();
    }
  }, 60_000);

  // A scan from a folder: Git's diff from a commit reads the rules of, and looks into, the submodule outside that folder
  // too, and prints none of it; the audit's diff, of the worktree with the index, does neither. Each row: the state, and
  // whether Git's own audit and scan from the folder answer.
  it.each<[string, (work: string) => void, [boolean, boolean]]>([
    ["whose `.gitmodules` Git cannot read", work => writeFileSync(join(work, ".gitmodules"), "[submodule\n"), [true, false]],
    ["whose submodule's `.git` file holds no `gitdir: `", work => writeFileSync(join(work, "sub", ".git"), "nothing\n"), [true, false]],
    ["whose submodule's worktree changed", work => writeFileSync(join(work, "sub", "inner.txt"), "two\n"), [true, true]],
    // The folder's own file `sub` changed, and Git reads no rule of the marked submodule `sub` outside the folder.
    ["whose marked submodule shares its path with a changed file of the folder, and whose `.gitmodules` Git cannot read", work => {
      git(work, "update-index", "--assume-unchanged", "sub");
      writeFileSync(join(work, ".gitmodules"), "[submodule\n");
      writeFileSync(join(work, "dir", "sub"), "changed\n");
    }, [true, true]],
  ])("and answer as Git's own diffs from a folder with no submodule below it, in a checkout %s", async (_what, made, answered) => {
    const root = await folder();
    const { work, gitfile, named } = await submoduleGitfile(root);
    await writeFile(gitfile, `${named}\n`);
    mkdirSync(join(work, "dir"));
    writeFileSync(join(work, "dir", "sub"), "one\n");
    git(work, "add", "dir");
    git(work, "commit", "--quiet", "-m", "dir");
    made(work);
    const dir = join(work, "dir");

    const answers = [await runGitWithoutBlocking(dir, AUDIT_DIFF), await runGitWithoutBlocking(dir, SCAN_DIFF)];
    const gits = [gitAnswer(dir, AUDIT_DIFF), gitAnswer(dir, SCAN_DIFF)];

    expect(answers.map(answer => [answer.status === 0, answer.stdout])).toEqual(gits);
    expect(gits.map(([ok]) => ok)).toEqual(answered);
  }, 60_000);

  // Git's diff looks into no submodule marked assume-unchanged or skip-worktree, so it opens no `.git` of one.
  it.for(["assume-unchanged", "skip-worktree"].flatMap(mark => MARKED_STATES.map(([what, made]) => [mark, what, made] as const)))(
    "and answer as Git's own diff for a submodule marked %s %s", { timeout: 60_000 }, async ([mark, _what, made], context) => {
      const root = await folder();
      const { work, gitfile, named } = await submoduleGitfile(root);
      await writeFile(gitfile, `${named}\n`);
      await writeFile(join(work, "sub", "inner.txt"), "two\n");
      const state = made(work, named);
      try {
        if (!state.held) context.skip();
        git(work, "update-index", `--${mark}`, "sub");
        const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

        expect(answers.map(answer => [answer.status, answer.stdout])).toEqual([[0, git(work, ...AUDIT_DIFF)], [0, git(work, ...SCAN_DIFF)]]);
      } finally {
        state.giveBack();
      }
    });

  // The `git status` Git's diff runs in a submodule reads no rule of a submodule inside it that is marked.
  it("and answer as Git's own diff for a submodule whose one submodule is marked and whose `.gitmodules` Git cannot read", async () => {
    const root = await folder();
    const lib = await repository(root, "lib", { "a.txt": "a\n" });
    const middle = await repository(root, "middle", { "m.txt": "m\n" });
    git(middle, "submodule", "add", "--quiet", lib, "leaf");
    git(middle, "commit", "--quiet", "-m", "leaf");
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", middle, "sub");
    git(work, "submodule", "update", "--quiet", "--init", "--recursive");
    git(work, "commit", "--quiet", "-m", "sub");
    const sub = join(work, "sub");
    git(sub, "update-index", "--assume-unchanged", "leaf");
    await writeFile(join(sub, ".gitmodules"), "[submodule\n");
    git(sub, "commit", "--quiet", "-m", "unreadable", "--", ".gitmodules");
    git(work, "add", "sub");
    await writeFile(join(sub, "leaf", "a.txt"), "changed\n");

    const audit = await runGitWithoutBlocking(work, AUDIT_DIFF);

    expect([audit.status, audit.stdout]).toEqual([0, git(work, ...AUDIT_DIFF)]);
  }, 60_000);

  it.skipIf(process.platform === "win32")("in the Git folder a submodule's `.git` named when Rerail looked, though it names a placed one by the status", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const { work, placed } = await submoduleWithPlacedCopy(root, marker);
    const gitfile = join(work, "sub", ".git");
    // A driver in the submodule's own config, so the guard lists it, and the `.git` changes between that and the status.
    git(work, "config", "--file", join(work, ".git", "modules", "sub", "config"), "filter.probe.clean", "cat");

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

  it.skipIf(process.platform === "win32")("in a Git folder taken away while Git listed its config and while Git listed that file, put back after each", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const { work, placed } = await submoduleWithPlacedCopy(root, marker);
    const kept = join(root, "kept");
    unlinkSync(join(work, "sub", ".git"));
    await writeFile(join(work, "sub", ".git"), "gitdir: ../placed\n");

    const audit = await afterSubmoduleListing(root, work, `mv '${kept}' '${placed}'`, `mv '${placed}' '${kept}'`, "--list --file");

    expect(existsSync(placed)).toBe(true);
    expect(runs(marker)).toBe(0);
    expect([audit.status, listed(audit.stdout)]).toEqual([0, ["sub"]]);
  }, 60_000);

  it.skipIf(process.platform === "win32")("in a Git folder taken away while Rerail listed its config, and list that submodule as changed", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const { work, placed } = await submoduleWithPlacedCopy(root, marker);
    unlinkSync(join(work, "sub", ".git"));
    await writeFile(join(work, "sub", ".git"), "gitdir: ../placed\n");

    const audit = await afterSubmoduleListing(root, work, ":", `mv '${placed}' '${join(root, "kept")}'`);

    expect(runs(marker)).toBe(0);
    expect([audit.status, listed(audit.stdout)]).toEqual([0, ["sub"]]);
  }, 60_000);

  it("and list a submodule whose own submodule's commit moved, not staged in it, as Git does", async () => {
    const root = await folder();
    const leaf = await repository(root, "leaf", { "leaf.txt": "one\n" });
    const middle = await repository(root, "middle", { "middle.txt": "one\n" });
    git(middle, "submodule", "add", "--quiet", leaf, "leaf");
    git(middle, "commit", "--quiet", "-m", "leaf");
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", middle, "sub");
    git(work, "submodule", "update", "--quiet", "--init", "--recursive");
    git(work, "commit", "--quiet", "-m", "submodule");
    git(join(work, "sub", "leaf"), "commit", "--quiet", "--allow-empty", "-m", "moved");

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

    expect(answers.map(answer => answer.stdout)).toEqual([git(work, ...AUDIT_DIFF), git(work, ...SCAN_DIFF)]);
    expect(answers.map(answer => listed(answer.stdout))).toEqual([["sub"], ["sub"]]);
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

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];

    expect(runs(marker)).toBe(0);
    expect(answers.map(answer => [answer.status, listed(answer.stdout)])).toEqual([[0, ["sub"]], [0, ["sub"]]]);
  }, 60_000);

  it("in a Git folder a writer of the worktree alone placed for a registered submodule that was not checked out", async () => {
    const root = await folder();
    const marker = join(root, "driver-ran");
    const { work, gitfile } = await placedSubmodule(root, marker);
    await writeFile(gitfile, "gitdir: ../placed\n");

    const answers = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF)];
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
      for (let call = 0; call < 40; call += 1) await runGitWithoutBlocking(work, AUDIT_DIFF);
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
 * that listing; and around the listing of that config file `alsoAround` names the rest of, when given.
 */
async function afterSubmoduleListing(root: string, work: string, command: string, before = ":", alsoAround?: string): Promise<{ status: number | null; stdout: string }> {
  const real = execFileSync("sh", ["-c", "command -v git"], { encoding: "utf8" }).trim();
  const bin = join(root, "bin");
  await mkdir(bin);
  const sub = `-C ${realpathSync.native(join(work, "sub"))} config -z`;
  const listings = [`${sub} --show-scope --list`, ...(alsoAround === undefined ? [] : [`${sub} ${alsoAround}`])];
  const around = (what: string): string => `case " $* " in ${listings.map(listing => `*" ${listing} "*`).join("|")}) ${what};; esac`;
  await writeFile(join(bin, "git"), ["#!/bin/sh", around(before), `'${real}' "$@"; code=$?`, around(command), "exit $code", ""].join("\n"));
  chmodSync(join(bin, "git"), 0o755);
  vi.stubEnv("PATH", `${bin}${delimiter}${process.env.PATH ?? ""}`);
  vi.resetModules();
  const fresh = await import("../../../src/engine/git.js");
  return await fresh.runGitWithoutBlocking(work, AUDIT_DIFF);
}

/**
 * A checkout `work` with a checked-out submodule `sub` whose `.git` file `gitfile`, which named its Git folder with
 * `named`, is taken away for the test to write: Git for Windows hides it, and Windows refuses to open a hidden file for
 * writing.
 */
async function submoduleGitfile(root: string): Promise<{ work: string; gitfile: string; named: string }> {
  const source = await repository(root, "source", { "inner.txt": "one\n" });
  const work = await repository(root, "work", { "top.txt": "top\n" });
  git(work, "submodule", "add", "--quiet", source, "sub");
  git(work, "commit", "--quiet", "-m", "submodule");
  const gitfile = join(work, "sub", ".git");
  const named = readFileSync(gitfile, "utf8").trimEnd();
  unlinkSync(gitfile);
  return { work, gitfile, named };
}

/** Marks each submodule the index of `work` records with `flag` (`--assume-unchanged` or `--skip-worktree`). */
function markSubmodules(work: string, flag: string): void {
  const gitlinks = git(work, "ls-files", "-z", "--stage").split("\0").filter(entry => entry.startsWith("160000 "));
  git(work, "update-index", flag, "--", ...gitlinks.map(entry => entry.slice(entry.indexOf("\t") + 1)));
}

/** What the test's own Git prints to its error stream for `git <args>` in `cwd`; undefined when it answers. */
function gitStderr(cwd: string, args: readonly string[]): string | undefined {
  try {
    execFileSync("git", args, { cwd, encoding: "utf8", stdio: "pipe" });
    return undefined;
  } catch (error) {
    return (error as { stderr: string }).stderr;
  }
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
  // A driver in its config, so the guard lists it before the diff.
  ["diff.ignoreSubmodules = all in a checkout whose config defines a driver", work => {
    git(work, "config", "diff.ignoreSubmodules", "all");
    git(work, "config", "filter.probe.clean", "cat");
  }],
  [".gitmodules ignore = all staged, with no .gitmodules in the worktree", async work => {
    setGitmodules(work, "all");
    git(work, "add", ".gitmodules");
    await rm(join(work, ".gitmodules"));
  }],
  [".gitmodules ignore = all committed, with no .gitmodules in the worktree or the index", async work => {
    setGitmodules(work, "all");
    git(work, "commit", "--quiet", "-m", "ignore all", ".gitmodules");
    git(work, "rm", "--quiet", "--cached", ".gitmodules");
    await rm(join(work, ".gitmodules"));
  }],
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
  // Git's diff and the `git status` it runs in a submodule look into no submodule marked so.
  ["every submodule marked assume-unchanged", work => markSubmodules(work, "--assume-unchanged")],
  ["every submodule marked skip-worktree", work => markSubmodules(work, "--skip-worktree")],
  ["the submodules inside the submodules that hold one marked assume-unchanged in one, skip-worktree in the other", work => {
    markSubmodules(join(work, "nested-modified"), "--assume-unchanged");
    markSubmodules(join(work, "nested-untracked"), "--skip-worktree");
  }],
  // Git reads no `.gitmodules` in conflict, so each submodule takes the base, whatever the worktree file says.
  [".gitmodules in conflict, with conflict markers in the worktree", async work => {
    await conflictedGitmodules(work, committed => `<<<<<<< ours\n${committed}=======\n${ignoringAll(committed)}>>>>>>> theirs\n`);
  }],
  [".gitmodules in conflict, resolved to ignore = all in the worktree and not staged", async work => {
    await conflictedGitmodules(work, ignoringAll);
  }],
  ["GIT_LITERAL_PATHSPECS=1 in the environment", () => void vi.stubEnv("GIT_LITERAL_PATHSPECS", "1")],
];

/** `.gitmodules` of `work` in conflict: the committed file at stages 1 to 3 of the index, and `text` of it in the worktree. */
async function conflictedGitmodules(work: string, text: (committed: string) => string): Promise<void> {
  const committed = readFileSync(join(work, ".gitmodules"), "utf8");
  const blob = git(work, "hash-object", "-w", ".gitmodules").trim();
  git(work, "update-index", "--force-remove", ".gitmodules");
  execFileSync("git", ["update-index", "--index-info"], { cwd: work, input: [1, 2, 3].map(stage => `100644 ${blob} ${stage}\t.gitmodules\n`).join("") });
  await writeFile(join(work, ".gitmodules"), text(committed));
}

/** The `.gitmodules` text `committed` with `ignore = all` for each submodule. */
function ignoringAll(committed: string): string {
  return committed.replaceAll(/^(\tpath = .*\n)/gmu, "$1\tignore = all\n");
}

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

    const rerail = [await runGitWithoutBlocking(work, AUDIT_DIFF), await runGitWithoutBlocking(work, SCAN_DIFF), await runGitWithoutBlocking(join(work, "deep"), AUDIT_DIFF)];
    const own = [git(work, ...AUDIT_DIFF), git(work, ...SCAN_DIFF), git(join(work, "deep"), ...AUDIT_DIFF)];

    expect(rerail.map(answer => answer.status)).toEqual([0, 0, 0]);
    expect(rerail.map(answer => answer.stdout)).toEqual(own);
  }, 120_000);

  it("in the defaults, which list a submodule with untracked files for the audit, and not for the scan", async () => {
    const root = await folder();
    const work = await submodulesInEachState(root);

    expect(listed((await runGitWithoutBlocking(work, AUDIT_DIFF)).stdout)).toEqual([
      "deep/moved", "deleted", "modified", "moved", "nested-modified", "nested-untracked", "staged", "untracked",
    ]);
    expect(listed((await runGitWithoutBlocking(work, SCAN_DIFF)).stdout)).toEqual(["deep/moved", "deleted", "modified", "moved", "nested-modified", "staged"]);
  }, 120_000);
});

/**
 * A clean checkout `work` with a clean checked-out submodule `sub` holding 300 empty folders no one may open (`shut`):
 * its status with untracked files lists no entry and warns of each, some 90 KB, past the 64 KiB Rerail reads of its
 * stdout.
 */
async function warningSubmodule(root: string): Promise<{ work: string; shut: string[] }> {
  const source = await repository(root, "source", { "inner.txt": "one\n" });
  const work = await repository(root, "work", { "top.txt": "top\n" });
  git(work, "submodule", "add", "--quiet", source, "sub");
  git(work, "commit", "--quiet", "-m", "submodule");
  const shut = Array.from({ length: 300 }, (_, at) => join(work, "sub", `shut-${String(at).padStart(3, "0")}-${"x".repeat(240)}`));
  for (const path of shut) mkdirSync(path, { mode: 0o000 });
  return { work, shut };
}

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
async function rerail(cwd: string, args: readonly string[]): Promise<[boolean, string]> {
  const answer = await runGitWithoutBlocking(cwd, args);
  return [answer.status === 0 && answer.error === undefined, answer.stdout];
}

/** The submodule `sub` of a `changedSubmodule` checkout with its Git folder made otherwise; the checkout to diff in. */
const GIT_FOLDERS: ReadonlyArray<readonly [string, (work: string, root: string) => Promise<string>]> = [
  ["whose HEAD Git does not take", async work => {
    writeFileSync(join(work, ".git", "modules", "sub", "HEAD"), "not a head\n");
    return work;
  }],
  ...["objects", "refs"].map(name => [`with no ${name} folder`, async (work: string) => {
    await removeFolder(join(work, ".git", "modules", "sub", name));
    return work;
  }] as const),
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
  ["whose HEAD is `ref:` and a tab before `refs/`, which Git takes", async work => {
    const head = join(work, ".git", "modules", "sub", "HEAD");
    writeFileSync(head, readFileSync(head, "utf8").replace(/^ref: /u, "ref:\t"));
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
  ["in a linked worktree of the submodule's own repository, whose `commondir` climbs out of it through a link", async work => {
    const commit = git(join(work, "sub"), "rev-parse", "HEAD").trim();
    const gitFolder = join(work, ".git", "modules", "sub");
    await removeFolder(join(work, "sub"));
    git(work, "config", "--file", join(gitFolder, "config"), "--unset", "core.worktree");
    git(work, "--git-dir", gitFolder, "worktree", "add", "--quiet", "--detach", join(work, "sub"), commit);
    await writeFile(join(work, "sub", "inner.txt"), "two\n");
    // On Linux and macOS the four `..` go up from where the link leads, to the repository; read as text, to the folder
    // above it, which holds none. Windows folds them as text, for Git too, so there both diffs fail alike.
    const linkedFolder = join(gitFolder, "worktrees", "sub");
    mkdirSync(join(linkedFolder, "a", "b"), { recursive: true });
    symlinkSync(join(linkedFolder, "a", "b"), join(linkedFolder, "ln"), "junction");
    writeFileSync(join(linkedFolder, "commondir"), "ln/../../../..\n");
    return work;
  }],
];

/** Each config file of a submodule's Git folder that holds no entry, and how to make it so. */
const NO_ENTRY: ReadonlyArray<readonly [string, (gitFolder: string) => void]> = [
  ["is empty", gitFolder => writeFileSync(join(gitFolder, "config"), "")],
  ["is gone", gitFolder => unlinkSync(join(gitFolder, "config"))],
  ["holds only a comment and a section header", gitFolder => writeFileSync(join(gitFolder, "config"), "# no entry\n[core]\n")],
  // Headers Git takes whole, which a reading of its syntax line by line could take for an entry.
  ["holds only a header whose subsection holds `]`", gitFolder => writeFileSync(join(gitFolder, "config"), '[filter "a]b"]\n')],
  ["holds only a header whose subsection holds an escaped quote", gitFolder => writeFileSync(join(gitFolder, "config"), '[filter "x\\"]y"]\n')],
  ["holds only a header holding `]` and a comment after it", gitFolder => writeFileSync(join(gitFolder, "config"), '[filter "a]b"] ; no entry\n')],
];

describe("Rerail's diffs answer as Git's own for each Git folder a submodule's `.git` names", () => {
  it.each(GIT_FOLDERS)("one %s", async (_name, made) => {
    const root = await folder();
    const work = await made(await changedSubmodule(root), root);

    expect([await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)]).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
  }, 60_000);

  it.each(NO_ENTRY)("one whose config file %s, which defines no driver, and list it when it changed, as Git does", async (_name, made) => {
    const root = await folder();
    const work = await changedSubmodule(root);
    made(join(work, ".git", "modules", "sub"));
    const changed = [await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)];
    await writeFile(join(work, "sub", "inner.txt"), "one\n");
    const clean = [await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)];

    expect(changed).toEqual([[true, "sub\0top.txt\0"], [true, "sub\0top.txt\0"]]);
    expect(clean).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
    expect(clean).toEqual([[true, "top.txt\0"], [true, "top.txt\0"]]);
  }, 60_000);

  it.skipIf(process.platform === "win32")("one whose HEAD is a symbolic link into refs/, as Git once wrote it, to a branch not made yet", async () => {
    const root = await folder();
    const work = await changedSubmodule(root);
    const head = join(work, ".git", "modules", "sub", "HEAD");
    unlinkSync(head);
    symlinkSync("refs/heads/unborn", head);

    const answers = [await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)];

    expect(answers).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
    expect(answers.map(([answered]) => answered)).toEqual([true, true]);
  }, 60_000);

  it("one whose config file sets a driver for the changed file, its header split from its key by blank lines", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const source = await repository(root, "source", { ".gitattributes": "*.txt filter=inner\n", "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    const config = join(work, ".git", "modules", "sub", "config");
    writeFileSync(config, `${readFileSync(config, "utf8")}[filter "inner"]\n\n\n\tclean = ${recording(marker)}\n`);
    await writeFile(join(work, "sub", "inner.txt"), "two\n");

    const answers = [await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)];

    expect(runs(marker)).toBe(0);
    expect(answers).toEqual([[true, "sub\0"], [true, "sub\0"]]);
  }, 60_000);

  it("one in a linked worktree of the submodule's own repository, whose repository's config is empty and whose Git folder holds a `config` Git does not read", async () => {
    const root = await folder();
    const work = await changedSubmodule(root);
    const commit = git(join(work, "sub"), "rev-parse", "HEAD").trim();
    const gitFolder = join(work, ".git", "modules", "sub");
    await removeFolder(join(work, "sub"));
    git(work, "config", "--file", join(gitFolder, "config"), "--unset", "core.worktree");
    git(work, "--git-dir", gitFolder, "worktree", "add", "--quiet", "--detach", join(work, "sub"), commit);
    writeFileSync(join(gitFolder, "config"), "");
    writeFileSync(join(gitFolder, "worktrees", "sub", "config"), "[core]\n\tbare = false\n");

    const answers = [await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)];

    expect(answers).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
    expect(answers).toEqual([[true, "top.txt\0"], [true, "top.txt\0"]]);
  }, 60_000);

  it.skipIf(process.platform === "win32")("one whose HEAD is a symbolic link outside refs/, which Git does not take, and start no Git in it", async () => {
    const root = await folder();
    const work = await changedSubmodule(root);
    const head = join(work, ".git", "modules", "sub", "HEAD");
    unlinkSync(head);
    symlinkSync("../HEAD", head);
    const trace = join(root, "trace");
    await mkdir(trace);
    vi.stubEnv("GIT_TRACE2_EVENT", trace);

    const answers = [await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)];

    vi.stubEnv("GIT_TRACE2_EVENT", undefined);
    // Each Git process writes its events to a file of its own; its `start` event holds its arguments.
    const events = readdirSync(trace).flatMap(name => readFileSync(join(trace, name), "utf8").split("\n").filter(line => line !== ""));
    const inSub = events.map(line => JSON.parse(line) as { event: string; argv?: string[] })
      .filter(event => event.event === "start" && event.argv?.some(arg => arg.endsWith(`${sep}sub`) || arg.endsWith("/sub")) === true);
    expect(answers).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
    expect([answers.map(([answered]) => answered), inSub]).toEqual([[false, false], []]);
  }, 60_000);

  it("one holding a submodule whose ignore rule Git refuses, which fails Git's own diff", async () => {
    const root = await folder();
    const leaf = await repository(root, "leaf", { "leaf.txt": "one\n" });
    const middle = await repository(root, "middle", { "middle.txt": "one\n" });
    git(middle, "submodule", "add", "--quiet", leaf, "leaf");
    git(middle, "commit", "--quiet", "-m", "leaf");
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", middle, "sub");
    git(work, "submodule", "update", "--quiet", "--init", "--recursive");
    git(work, "commit", "--quiet", "-m", "submodule");
    git(join(work, "sub"), "config", "submodule.leaf.ignore", "sometimes");

    const answers = [await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)];

    expect(answers).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
    expect(answers.map(([answered]) => answered)).toEqual([false, false]);
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

    const answers = [await rerail(work, AUDIT_DIFF), await rerail(work, SCAN_DIFF)];

    expect(answers).toEqual([own(work, AUDIT_DIFF), own(work, SCAN_DIFF)]);
    expect(answers).toEqual([[true, "sub\0"], [true, "sub\0"]]);
  }, 60_000);
});

/**
 * Folders a diff runs in where the path Rerail is given leads, read as text, to another repository's `.git` than the
 * one Git finds, and how to make each; the folder to diff in. A link is a junction on Windows.
 */
const DISCOVERED: ReadonlyArray<readonly [string, (root: string) => Promise<string>]> = [
  ["a link in a repository whose index records no submodule, to a folder of a checkout whose submodule there changed", async root => {
    const source = await repository(root, "source", { "inner.txt": "one\n" });
    const work = await repository(root, "work", { "top.txt": "top\n" });
    git(work, "submodule", "add", "--quiet", source, "deep/sub");
    git(work, "commit", "--quiet", "-m", "submodule");
    await writeFile(join(work, "deep", "sub", "inner.txt"), "two\n");
    const outer = await repository(root, "outer", { "outer.txt": "outer\n" });
    symlinkSync(join(work, "deep"), join(outer, "link"), "junction");
    return join(outer, "link");
  }],
  ["a checkout whose `.git` file names its Git folder through a link and `..`, where the text alone leads to a repository with no submodule", async root => {
    const work = await changedSubmodule(root);
    const store = join(root, "a", "store.git");
    mkdirSync(join(root, "a", "b"), { recursive: true });
    renameSync(join(work, ".git"), store);
    git(store, "config", "--file", join(store, "modules", "sub", "config"), "core.worktree", join(work, "sub"));
    // Git for Windows hides a submodule's `.git` file, and Windows refuses to open a hidden file for writing.
    unlinkSync(join(work, "sub", ".git"));
    await writeFile(join(work, "sub", ".git"), `gitdir: ${join(store, "modules", "sub").replaceAll("\\", "/")}\n`);
    symlinkSync(join(root, "a", "b"), join(work, "ln"), "junction");
    // On Linux and macOS `..` goes up from where the link leads, to `a/store.git`. Windows folds it as text, for Git
    // too, to the bare `work/store.git`, and there both diffs fail alike.
    await writeFile(join(work, ".git"), "gitdir: ln/../store.git\n");
    git(root, "init", "--quiet", "--bare", join(work, "store.git"));
    cpSync(join(root, "source", ".git", "index"), join(work, "store.git", "index"));
    return work;
  }],
];

describe("Rerail's diffs read the index Git's own diff reads", () => {
  it.each(DISCOVERED)("in %s", async (_name, made) => {
    const cwd = await made(await folder());

    const answers = [await rerail(cwd, AUDIT_DIFF), await rerail(cwd, SCAN_DIFF)];

    expect(answers).toEqual([own(cwd, AUDIT_DIFF), own(cwd, SCAN_DIFF)]);
  }, 60_000);
});

/** What a call made in a Rerail loaded afresh started: each Git's file, arguments and time limit, and the call's result. */
interface Started<T> {
  readonly gits: ReadonlyArray<{ readonly file: string; readonly args: readonly string[]; readonly limit: number | undefined }>;
  readonly answer: T;
}

/**
 * `call` in a Rerail loaded afresh, which asks Git's version at its first call that reads worktree files. Each Git it
 * starts, blocking or not, answers as itself, or `git version` as `version` when given, and as if it had taken `step` ms.
 */
async function startedBy<T>(call: (fresh: typeof import("../../../src/engine/git.js")) => T | Promise<T>, version?: string, step = 2_500): Promise<Started<T>> {
  vi.resetModules();
  const fresh = await import("../../../src/engine/git.js");
  const started = await import("node:child_process");
  const now = performance.now.bind(performance);
  let used = 0;
  const clock = vi.spyOn(performance, "now").mockImplementation(() => now() + used);
  const gits: Array<{ file: string; args: readonly string[]; limit: number | undefined }> = [];
  const asked = (file: string, args: readonly string[], limit: number | undefined): void => {
    if (!args.includes("-C")) return;
    gits.push({ file, args, limit });
    used += step;
  };
  const versionAsked = (args: readonly string[]): boolean => version !== undefined && args.at(-1) === "version";
  vi.mocked(started.spawnSync).mockImplementation(((file: string, args: readonly string[], options: childProcess.SpawnSyncOptions) => {
    const result = versionAsked(args)
      ? { pid: 0, output: [], stdout: version, stderr: "", status: 0, signal: null }
      : actualChildProcess.spawnSync(file, args, options);
    asked(file, args, options.timeout);
    return result;
  }) as unknown as typeof childProcess.spawnSync);
  vi.mocked(started.spawn).mockImplementation(((file: string, args: readonly string[], options: childProcess.SpawnOptions) => {
    // What was left of a 20 s limit when a Git that does not block started.
    asked(file, args, 20_000 - used);
    return versionAsked(args)
      ? actualChildProcess.spawn(process.execPath, ["-e", `process.stdout.write(${JSON.stringify(version)})`], options)
      : actualChildProcess.spawn(file, args, options);
  }) as unknown as typeof childProcess.spawn);
  try {
    return { answer: await call(fresh), gits };
  } finally {
    vi.mocked(started.spawnSync).mockImplementation(actualChildProcess.spawnSync);
    vi.mocked(started.spawn).mockImplementation(actualChildProcess.spawn);
    clock.mockRestore();
  }
}

/** Whether the `at`th Git of a call with a 20 s limit, each before it taking 2.5 s, started with what was left. */
const leftAt = ({ limit }: { readonly limit: number | undefined }, at: number): boolean =>
  limit !== undefined && limit > 18_000 - 2_500 * at && limit <= 20_000 - 2_500 * at;

/** A checkout `work` with the checked-out submodules `names`, each with `inner.txt` changed. */
async function changedSubmodules(root: string, names: readonly string[]): Promise<string> {
  const source = await repository(root, "source", { "inner.txt": "one\n" });
  const work = await repository(root, "work", { "top.txt": "top\n" });
  for (const name of names) git(work, "submodule", "add", "--quiet", source, name);
  git(work, "commit", "--quiet", "-m", "submodules");
  for (const name of names) await writeFile(join(work, name, "inner.txt"), "two\n");
  return work;
}

/** A program that waits `wait` ms, then runs the program and arguments after it with its own input and output. */
const LATE = String.raw`
const { spawnSync } = require("node:child_process");
const [wait, file, ...args] = process.argv.slice(2);
setTimeout(() => process.exit(spawnSync(file, args, { stdio: "inherit" }).status ?? 1), Number(wait));`;

/**
 * `call` with each Git that does not block started `wait(args)` ms late, through {@link LATE}; and the most such Gits
 * that ran at once.
 */
async function startedLate<T>(root: string, wait: (args: readonly string[]) => number, call: () => Promise<T>): Promise<{ answer: T; most: number }> {
  const late = join(root, "late.cjs");
  writeFileSync(late, LATE);
  const started = await import("node:child_process");
  let running = 0;
  let most = 0;
  vi.mocked(started.spawn).mockImplementation(((file: string, args: readonly string[], options: childProcess.SpawnOptions) => {
    // Only a Git starts late: on Windows ending a Git's process tree starts `taskkill` too.
    if (!args.includes("-C")) return actualChildProcess.spawn(file, args, options);
    const child = actualChildProcess.spawn(process.execPath, [late, String(wait(args)), file, ...args], options);
    running += 1;
    most = Math.max(most, running);
    child.once("exit", () => {
      running -= 1;
    });
    return child;
  }) as unknown as typeof childProcess.spawn);
  try {
    return { answer: await call(), most };
  } finally {
    vi.mocked(started.spawn).mockImplementation(actualChildProcess.spawn);
  }
}

describe("Rerail's diff of a checkout with submodules keeps the call's one time limit", () => {
  it("as each Git it starts gets what is left of it, and none starts once it is spent", async () => {
    const root = await folder();
    const work = await changedSubmodules(root, ["one", "two", "three"]);

    // Each Git as if it took 3 s of the 20 s.
    const { gits, answer } = await startedBy(async fresh => await fresh.runGitWithoutBlocking(work, AUDIT_DIFF, { timeout: 20_000 }), undefined, 3_000);
    // A start that runs past its own time limit, for the error Node gives it on this system.
    const ranOut = actualChildProcess.spawnSync(process.execPath, ["-e", "setTimeout(() => {}, 5000)"], { timeout: 50 }).error as NodeJS.ErrnoException | undefined;

    // Git's version, the diff, the index's submodules, the top folder and the config listing, then the submodules'
    // statuses, asked at once, each with what was left: two, and the third not started.
    expect(gits.map(({ limit }, at) => limit !== undefined && limit > 18_000 - 3_000 * at && limit <= 20_000 - 3_000 * at)).toEqual(Array(7).fill(true));
    expect(gits.slice(5).map(({ args }) => args.slice(-5))).toEqual(Array(2).fill(["status", "--porcelain=2", "-z", "--no-renames", "--ignore-submodules=dirty"]));
    expect([answer.status, answer.stdout, answer.stderr, answer.error?.message]).toEqual([null, "", "", `spawnSync ${gits[0]?.file} ETIMEDOUT`]);
    expect(answer.error).toMatchObject({ code: "ETIMEDOUT", errno: ranOut?.errno, syscall: `spawnSync ${gits[0]?.file}`, path: gits[0]?.file });
    // The status of the submodule asked last, whichever its index read let through last.
    const spawnargs = (answer.error as unknown as { spawnargs: string[] }).spawnargs;
    expect([spawnargs.slice(0, 2), spawnargs.at(-7), /[\\/](?:one|two|three)$/u.test(spawnargs.at(-6) ?? ""), spawnargs.slice(-5)]).toEqual([
      ["-c", "core.fsmonitor=false"], "-C", true, ["status", "--porcelain=2", "-z", "--no-renames", "--ignore-submodules=dirty"],
    ]);
  }, 60_000);

  it("and answers where one Git would, with submodules whose every Git takes a tenth of it", async () => {
    const root = await folder();
    const work = await changedSubmodules(root, ["one", "two", "three", "four"]);
    await writeFile(join(work, "top.txt"), "changed\n");
    // Git's version, asked once in a process, is asked first.
    await runGitWithoutBlocking(work, AUDIT_DIFF);

    const { answer } = await startedLate(root, () => 1_000, async () => await runGitWithoutBlocking(work, AUDIT_DIFF));

    // Asked in turn, the 16 Gits of the call (4 for the checkout, 3 for each submodule) would take 16 s.
    expect([answer.status, answer.error?.message, listed(answer.stdout)]).toEqual([0, undefined, ["four", "one", "three", "top.txt", "two"]]);
  }, 60_000);

  it("and runs no more Gits at once than the machine has processors", async () => {
    const root = await folder();
    const names = Array.from({ length: availableParallelism() + 2 }, (_, at) => `sub${at}`);
    const work = await changedSubmodules(root, names);

    const { answer, most } = await startedLate(root, () => 200, async () => await runGitWithoutBlocking(work, AUDIT_DIFF, { timeout: 60_000 }));

    expect([answer.status, listed(answer.stdout)]).toEqual([0, [...names].sort()]);
    expect([most > 1, most <= availableParallelism()]).toEqual([true, true]);
  }, 120_000);

  it("and ends at it, failing as a Git past its limit, when a submodule's Git does not answer", async () => {
    const root = await folder();
    const work = await changedSubmodules(root, ["one"]);
    await runGitWithoutBlocking(work, AUDIT_DIFF);

    const began = performance.now();
    const { answer } = await startedLate(root, args => args.includes("status") ? 600_000 : 0, async () => await runGitWithoutBlocking(work, AUDIT_DIFF, { timeout: 2_000 }));
    const ms = performance.now() - began;

    expect(answer.error).toMatchObject({ code: "ETIMEDOUT", message: expect.stringMatching(/^spawnSync .* ETIMEDOUT$/u) });
    expect([answer.status, ms >= 2_000, ms < 8_000]).toEqual([null, true, true]);
  }, 60_000);

  it("and so does the session walk's listing, which does not block: its Gits share the read's 10 s", async () => {
    const root = await folder();
    const work = withDriver(await repository(root, "work", { "top.txt": "top\n" }));

    const { gits, answer } = await startedBy(async fresh => await fresh.gitModifiedOrUntrackedListing(work), undefined, 5_000);

    // Git's version and the config listing take the whole 10 s: the listing of changed files is not started.
    expect([gits.map(({ args }) => args.at(-1)), answer]).toEqual([["version", "--list"], { ended: "timed-out" }]);
  }, 60_000);

  it.each([["config listing", "--list"], ["listing of changed files", "--modified"]])("and the walk's %s gets only what is left of the read's 10 s", async (_name, hangs) => {
    const root = await folder();
    const work = withDriver(await repository(root, "work", { "top.txt": "top\n" }));
    vi.resetModules();
    const fresh = await import("../../../src/engine/git.js");
    const started = await import("node:child_process");
    const now = performance.now.bind(performance);
    let used = 0;
    const clock = vi.spyOn(performance, "now").mockImplementation(() => now() + used);
    // Each Git before the one that hangs as if it took 4 s; in that one's place, a program that prints nothing and
    // stays: the read ends it at its first look past what was left (6 s for the config listing, 2 s for the other).
    vi.mocked(started.spawn).mockImplementation(((file: string, args: readonly string[], options: childProcess.SpawnOptions) => {
      const stays = args.at(-1) === hangs || args.includes(hangs);
      used += stays ? 0 : 4_000;
      return stays ? actualChildProcess.spawn(process.execPath, ["-e", "process.stdin.resume()"], options) : actualChildProcess.spawn(file, args, options);
    }) as unknown as typeof childProcess.spawn);
    const began = now();
    try {
      const read = fresh.gitModifiedOrUntrackedListing(work);
      await new Promise(settle => setTimeout(settle, 500));
      // Past what was left on the clock, and 4 s short of a limit of 10 s from the start of the Git that stays.
      used += 6_000;

      expect(await read).toEqual({ ended: "timed-out" });
      expect(now() - began).toBeLessThan(3_000);
    } finally {
      vi.mocked(started.spawn).mockImplementation(actualChildProcess.spawn);
      clock.mockRestore();
    }
  }, 60_000);

  it.each([["status", "--porcelain=2", "-z"], ["ls-files", "-z", "--modified"]])("and so does a guarded `git %s`", async (...args: string[]) => {
    const root = await folder();
    const work = withDriver(await repository(root, "work", { "top.txt": "top\n" }));

    const { gits, answer } = await startedBy(fresh => fresh.runGit(work, args, { timeout: 20_000 }));

    // Git's version, the config listing, and the call.
    expect([gits.map(leftAt), answer.status]).toEqual([[true, true, true], 0]);
  }, 60_000);

  it("and so does a call with a Git too old for the guard", async () => {
    const root = await folder();
    const work = await repository(root, "work", { "top.txt": "top\n" });
    const quiet = vi.spyOn(process.stderr, "write").mockImplementation(() => true);

    const { gits, answer } = await startedBy(async fresh => await fresh.runGitWithoutBlocking(work, AUDIT_DIFF, { timeout: 20_000 }), "git version 2.30.2\n");
    quiet.mockRestore();

    // Git's version, then the diff as it ran before the guard.
    expect([gits.map(leftAt), gits.map(({ args }) => args.at(-1)), answer.status]).toEqual([[true, true], ["version", "--"], 0]);
  }, 60_000);
});
