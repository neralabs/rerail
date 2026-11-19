import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, existsSync, readFileSync } from "node:fs";
import { mkdir, mkdtemp, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import { gitModifiedOrUntrackedPaths, runGit } from "../../src/engine/git.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  vi.restoreAllMocks();
  vi.resetModules();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/** The test's own Git, which runs every driver its config names. */
function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "protocol.file.allow=always", ...args], {
    cwd, encoding: "utf8",
  });
}

async function folder(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-git-drivers-"));
  roots.push(root);
  return root;
}

/** A repository at `root/name` whose config `config` sets, with one commit of `files` made through it. */
async function repository(root: string, name: string, files: Record<string, string>, config: Record<string, string> = {}): Promise<string> {
  const path = join(root, name);
  await mkdir(path);
  git(path, "init", "--quiet");
  for (const [key, value] of Object.entries({ "core.autocrlf": "false", ...config })) git(path, "config", key, value);
  for (const [file, text] of Object.entries(files)) await writeFile(join(path, file), text);
  git(path, "add", ".");
  git(path, "commit", "--quiet", "-m", "seed");
  return path;
}

/** A stand-in driver command: it writes a line to `marker`, a file of the test's own, each time it runs, then runs `then`. */
function recording(marker: string, then = "cat"): string {
  return `echo ran >> '${marker.replaceAll("\\", "/")}'; ${then}`;
}

/** `command` as a config file value: quoted, since Git reads an unquoted `;` as the start of a comment. */
function configValue(command: string): string {
  return `"${command.replaceAll("\\", "\\\\").replaceAll('"', String.raw`\"`)}"`;
}

function runs(marker: string): number {
  return existsSync(marker) ? readFileSync(marker, "utf8").split("\n").filter(line => line !== "").length : 0;
}

const sorted = (paths: readonly string[]): string[] => [...paths].sort();
const listed = (stdout: string): string[] => sorted(stdout.split("\0").filter(path => path !== ""));

/** Makes `path` look touched: a new modification time, the same bytes and size. */
async function touch(path: string): Promise<void> {
  const later = new Date(Date.now() + 60_000);
  await utimes(path, later, later);
}

/**
 * Dates `paths` an hour back, before the index Git writes next: an entry no older than its index is racily clean, and
 * Git compares its bytes as it compares a touched file's.
 */
async function aged(paths: readonly string[]): Promise<void> {
  const earlier = new Date(Date.now() - 3_600_000);
  await Promise.all(paths.map(path => utimes(path, earlier, earlier)));
}

/** The audit's `git diff` (`audit-control.ts`) and the scan's (`headless-scan.ts`), as each runs it. */
const AUDIT_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"];
const SCAN_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--diff-filter=ACMRTUXB", "-z", "HEAD", "--"];
/** The session walk's listing of changed files (`gitModifiedOrUntrackedPaths`). */
const WALK_LISTING = ["ls-files", "-z", "--modified", "--others", "--exclude-standard"];

describe("Rerail's Git calls start no filter program only a repository's own config names", () => {
  it("the audit's and the scan's diff list a file rewritten at the same size without starting its filter program", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const work = await repository(root, "work", { ".gitattributes": "*.txt filter=probe\n*.md filter=a=b\n", "notes.txt": "one\n", "read.md": "aa\n" });
    git(work, "config", "filter.probe.clean", recording(marker));
    git(work, "config", "filter.probe.required", "true");
    git(work, "config", "filter.a=b.clean", recording(marker));
    // The same size, other bytes: Git compares the bytes, through the driver.
    await writeFile(join(work, "notes.txt"), "two\n");
    await writeFile(join(work, "read.md"), "bb\n");

    const audit = runGit(work, AUDIT_DIFF);
    const scan = runGit(work, SCAN_DIFF);

    expect(runs(marker)).toBe(0);
    expect([audit.status, scan.status]).toEqual([0, 0]);
    expect([listed(audit.stdout), listed(scan.stdout)]).toEqual([["notes.txt", "read.md"], ["notes.txt", "read.md"]]);
  }, 60_000);

  it("the session walk's listing of changed files starts no filter program", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const work = await repository(root, "work", { ".gitattributes": "*.txt filter=probe\n", "notes.txt": "one\n" });
    git(work, "config", "filter.probe.clean", recording(marker));
    await writeFile(join(work, "notes.txt"), "two\n");

    const firstSeen = await gitModifiedOrUntrackedPaths(work);

    expect(runs(marker)).toBe(0);
    expect(firstSeen).toEqual(["notes.txt"]);
  }, 60_000);

  it("runs the person's own driver where the repository defines one of the same name, and answers as their Git does", async () => {
    const root = await folder();
    const [own, repositorys] = [join(root, "own-ran"), join(root, "repository-ran")];
    const person = join(root, "person.gitconfig");
    await writeFile(person, `[filter "shared"]\n\tclean = ${configValue(recording(own))}\n`);
    vi.stubEnv("GIT_CONFIG_GLOBAL", person);
    const work = await repository(root, "work", { ".gitattributes": "*.txt filter=shared\n", "notes.txt": "one\n" });
    git(work, "config", "filter.shared.clean", recording(repositorys));
    await touch(join(work, "notes.txt"));
    const ownBefore = runs(own);

    const audit = runGit(work, AUDIT_DIFF);

    expect(runs(repositorys)).toBe(0);
    expect(runs(own)).toBeGreaterThan(ownBefore);
    expect(listed(audit.stdout)).toEqual([]);
  }, 60_000);

  it("starts no driver and answers as Git does when the person's environment sets GIT_CONFIG, which only `git config` reads", async () => {
    const root = await folder();
    const marker = join(root, "filter-ran");
    const work = await repository(root, "work", { ".gitattributes": "*.txt filter=probe\n", "notes.txt": "one\n" });
    git(work, "config", "filter.probe.clean", recording(marker));
    await writeFile(join(work, "notes.txt"), "two\n");
    const other = join(root, "other.gitconfig");
    await writeFile(other, "[core]\n\tquotepath = false\n");

    vi.stubEnv("GIT_CONFIG", other);
    const audit = runGit(work, AUDIT_DIFF);
    const firstSeen = await gitModifiedOrUntrackedPaths(work);
    vi.stubEnv("GIT_CONFIG", join(root, "missing.gitconfig"));
    const scan = runGit(work, SCAN_DIFF);

    expect(runs(marker)).toBe(0);
    expect([audit.status, scan.status]).toEqual([0, 0]);
    expect([listed(audit.stdout), firstSeen, listed(scan.stdout)]).toEqual([["notes.txt"], ["notes.txt"], ["notes.txt"]]);
  }, 60_000);

  it.skipIf(process.platform === "win32")("makes no call whose config Git cannot list, and its result holds none of what the listing printed", async () => {
    const root = await folder();
    const real = execFileSync("sh", ["-c", "command -v git"], { encoding: "utf8" }).trim();
    const bin = join(root, "bin");
    await mkdir(bin);
    // A Git whose config listing prints one entry and then fails, as a listing does that meets an include being renamed.
    await writeFile(join(bin, "git"), `#!/bin/sh\ncase " $* " in *" --show-scope --list "*) printf 'local\\0core.bare\\nfalse\\0'; exit 128;; esac\nexec '${real}' "$@"\n`);
    chmodSync(join(bin, "git"), 0o755);
    const work = await repository(root, "work", { "notes.txt": "one\n" });
    await writeFile(join(work, "notes.txt"), "two\n");
    vi.stubEnv("PATH", `${bin}${delimiter}${process.env.PATH ?? ""}`);
    vi.resetModules();
    const fresh = await import("../../src/engine/git.js");

    const audit = fresh.runGit(work, AUDIT_DIFF);
    const walk = await fresh.gitModifiedOrUntrackedPaths(work);

    expect([audit.status, audit.stdout]).toEqual([128, ""]);
    expect(walk).toEqual([]);
  }, 60_000);
});

/** Where a test repository's filter driver `upper` is defined: the person's own config, Git's system config or the repository's. */
type DriverOwner = "person" | "system" | "repository";

/**
 * A repository whose `*.txt` files go through the driver `upper` (it stores them upper-cased and writes a line to
 * `marker` each run), defined where `owner` says, with a file in each state a listing tells apart.
 */
async function driverRepository(owner: DriverOwner): Promise<{ work: string; marker: string }> {
  const root = await folder();
  const marker = join(root, "driver-ran");
  const driver = `[filter "upper"]\n\tclean = ${configValue(recording(marker, "tr a-z A-Z"))}\n`;
  if (owner !== "repository") {
    const config = join(root, `${owner}.gitconfig`);
    await writeFile(config, driver);
    vi.stubEnv(owner === "person" ? "GIT_CONFIG_GLOBAL" : "GIT_CONFIG_SYSTEM", config);
    vi.stubEnv("GIT_CONFIG_NOSYSTEM", undefined);
  }
  const work = join(root, "work");
  await mkdir(join(work, "sub"), { recursive: true });
  git(work, "init", "--quiet");
  if (owner === "repository") await writeFile(join(work, ".git", "config"), `${readFileSync(join(work, ".git", "config"), "utf8")}${driver}`);
  git(work, "config", "core.autocrlf", "input");
  await writeFile(join(work, ".git", "info", "exclude"), "*.log\n");
  const files = {
    ".gitattributes": "*.txt filter=upper\n", "untouched.txt": "one\n", "touched.txt": "one\n", "same.txt": "one\n", "resized.txt": "one\n",
    "staged.txt": "one\n", "deleted.txt": "one\n", "lines.md": "x\r\n", "sub/inner.txt": "one\n",
  };
  for (const [name, text] of Object.entries(files)) await writeFile(join(work, name), text);
  await aged(Object.keys(files).map(name => join(work, name)));
  git(work, "add", ".");
  git(work, "commit", "--quiet", "-m", "seed");
  await settle(work);
  return { work, marker };
}

/** Puts each file of {@link driverRepository} in its state: touched, rewritten at the same size or another, staged, deleted, new. */
async function settle(work: string): Promise<void> {
  await Promise.all(["touched.txt", "lines.md", "sub/inner.txt"].map(name => touch(join(work, name))));
  await writeFile(join(work, "same.txt"), "two\n");
  await writeFile(join(work, "resized.txt"), "three\n");
  await writeFile(join(work, "staged.txt"), "four\n");
  await aged([join(work, "staged.txt")]);
  git(work, "add", "staged.txt");
  await rm(join(work, "deleted.txt"));
  await writeFile(join(work, "new.txt"), "new\n");
  await writeFile(join(work, "ignored.log"), "log\n");
}

/** What Rerail's three listings answer in `cwd`: the audit's diff, the scan's, and the session walk's changed files. */
async function rerailListings(cwd: string): Promise<string[][]> {
  return [listed(runGit(cwd, AUDIT_DIFF).stdout), listed(runGit(cwd, SCAN_DIFF).stdout), sorted(await gitModifiedOrUntrackedPaths(cwd) ?? [])];
}

/** What Git's own three listings answer in `cwd`, running every driver the config names. */
function gitListings(cwd: string): string[][] {
  return [listed(git(cwd, ...AUDIT_DIFF)), listed(git(cwd, ...SCAN_DIFF)), listed(git(cwd, ...WALK_LISTING))];
}

describe("Rerail's listings of changed files answer as the person's Git does", () => {
  it.each(["person", "system"] as const)("with the %s's own driver, which runs as their Git runs it", async owner => {
    const { work, marker } = await driverRepository(owner);
    const before = runs(marker);
    const rerail = await rerailListings(work);
    const rerailRuns = runs(marker) - before;

    expect(rerailRuns).toBeGreaterThan(0);
    expect(rerail).toEqual(gitListings(work));
    expect(rerail[0]).toEqual(["resized.txt", "same.txt"]);
  }, 60_000);

  it("with a driver only the repository defines, which starts no program: a touched file under it is listed as changed", async () => {
    const { work, marker } = await driverRepository("repository");
    const before = runs(marker);
    const rerail = await rerailListings(work);
    const rerailRuns = runs(marker) - before;

    expect(rerailRuns).toBe(0);
    // The one difference: the person's Git runs the repository's program, which maps the touched files' bytes to the
    // index's; with no program run they differ from it.
    const touched = ["sub/inner.txt", "touched.txt"];
    expect(rerail).toEqual(gitListings(work).map(list => sorted([...list, ...touched])));
  }, 60_000);

  it("from a folder below the top, in a linked worktree and before the first commit", async () => {
    const { work } = await driverRepository("person");
    await writeFile(join(work, "sub", "inner.txt"), "two\n");
    const below = await rerailListings(join(work, "sub"));
    const linked = join(work, "..", "linked");
    git(work, "worktree", "add", "--quiet", linked, "HEAD");
    await writeFile(join(linked, "same.txt"), "two\n");
    await touch(join(linked, "touched.txt"));
    const inLinked = await rerailListings(linked);
    const unborn = join(work, "..", "unborn");
    await mkdir(unborn);
    git(unborn, "init", "--quiet");
    await writeFile(join(unborn, "a.txt"), "a\n");
    git(unborn, "add", "a.txt");
    await writeFile(join(unborn, "a.txt"), "b\n");

    expect(below).toEqual(gitListings(join(work, "sub")));
    expect(below[0]).toEqual(["inner.txt"]);
    expect(inLinked).toEqual(gitListings(linked));
    expect(listed(runGit(unborn, AUDIT_DIFF).stdout)).toEqual(listed(git(unborn, ...AUDIT_DIFF)));
    expect(sorted(await gitModifiedOrUntrackedPaths(unborn) ?? [])).toEqual(listed(git(unborn, ...WALK_LISTING)));
  }, 60_000);

  it("with Git LFS set up as the person sets it up, which cleans and smudges as their Git runs it", async ({ skip }) => {
    skip(spawnSync("git", ["lfs", "version"]).status !== 0, "Git LFS is not installed here");
    const root = await folder();
    const person = join(root, "person.gitconfig");
    await writeFile(person, '[filter "lfs"]\n\tclean = git-lfs clean -- %f\n\tsmudge = git-lfs smudge -- %f\n\tprocess = git-lfs filter-process\n\trequired = true\n');
    vi.stubEnv("GIT_CONFIG_GLOBAL", person);
    const work = join(root, "work");
    await mkdir(work);
    git(work, "init", "--quiet");
    const files = { ".gitattributes": "*.bin filter=lfs diff=lfs merge=lfs -text\n", "touched.bin": "one\n", "same.bin": "one\n", "resized.bin": "one\n", "untouched.bin": "one\n" };
    for (const [name, text] of Object.entries(files)) await writeFile(join(work, name), text);
    git(work, "add", ".");
    git(work, "commit", "--quiet", "-m", "seed");
    await touch(join(work, "touched.bin"));
    await writeFile(join(work, "same.bin"), "two\n");
    await writeFile(join(work, "resized.bin"), "three\n");

    const rerail = await rerailListings(work);
    // The person's own Git still stores a pointer and checks the file out whole: Rerail's calls changed no config.
    const stored = git(work, "cat-file", "-p", "HEAD:untouched.bin");
    await rm(join(work, "untouched.bin"));
    git(work, "checkout", "--", "untouched.bin");

    expect(rerail).toEqual(gitListings(work));
    expect(rerail[0]).toEqual(["resized.bin", "same.bin"]);
    expect(stored).toMatch(/^version https:\/\/git-lfs\.github\.com\/spec\/v1\n/u);
    expect(readFileSync(join(work, "untouched.bin"), "utf8")).toBe("one\n");
  }, 60_000);
});

/**
 * A `git` in `root/bin` that answers as Git `version` does where Rerail's guard needs a newer one: it names that
 * version, ignores the command-line config of `GIT_CONFIG_COUNT` (Git 2.31), and before Git 2.26 refuses
 * `--show-scope` as an unknown option. Every other call runs the Git this test found first, as it is.
 */
async function olderGit(root: string, version: string): Promise<string> {
  const real = execFileSync("sh", ["-c", "command -v git"], { encoding: "utf8" }).trim();
  const bin = join(root, "bin");
  await mkdir(bin);
  const minor = Number(version.split(".")[1]);
  const refusesScope = minor < 26 ? "--show-scope) echo \"error: unknown option \\`show-scope'\" >&2; exit 129;;" : "";
  const ignoresCount = "for name in $(env | sed -n 's/^\\(GIT_CONFIG_\\(COUNT\\|KEY_[0-9]*\\|VALUE_[0-9]*\\)\\)=.*/\\1/p'); do unset \"$name\"; done";
  const program = [
    "#!/bin/sh",
    "for argument in \"$@\"; do",
    `  case "$argument" in version|--version) echo "git version ${version}"; exit 0;; ${refusesScope} esac`,
    "done",
    minor < 31 ? ignoresCount : "",
    `exec '${real}' "$@"`,
  ].join("\n");
  await writeFile(join(bin, "git"), `${program}\n`);
  chmodSync(join(bin, "git"), 0o755);
  return bin;
}

/** A checkout whose `notes.txt` was rewritten at the same size under a driver its own config names. */
async function checkoutWithDriver(root: string): Promise<{ work: string; marker: string }> {
  const work = join(root, "work");
  const marker = join(root, "filter-ran");
  await mkdir(work);
  const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", ...args], { cwd: work, encoding: "utf8" });
  git("init", "--quiet");
  await writeFile(join(work, ".gitattributes"), "*.txt filter=probe\n");
  await writeFile(join(work, "notes.txt"), "one\n");
  git("add", ".");
  git("commit", "--quiet", "-m", "seed");
  git("config", "filter.probe.clean", `echo ran >> '${marker}'; cat`);
  await writeFile(join(work, "notes.txt"), "two\n");
  return { work, marker };
}

describe.skipIf(process.platform === "win32")("on a Git older than the guard works with", () => {
  it.each(["2.25.1", "2.30.2"])("Git %s: each call answers as that Git answers it, and Rerail says why once", async version => {
    const root = await folder();
    vi.stubEnv("PATH", `${await olderGit(root, version)}${delimiter}${process.env.PATH ?? ""}`);
    vi.resetModules();
    const { gitModifiedOrUntrackedPaths, runGit } = await import("../../src/engine/git.js");
    const { work } = await checkoutWithDriver(root);
    const said = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const git = (...args: string[]) => execFileSync("git", ["-c", "core.fsmonitor=false", ...args], { cwd: work, encoding: "utf8" });

    const answers = [runGit(work, AUDIT_DIFF), runGit(work, SCAN_DIFF)];
    const walk = await gitModifiedOrUntrackedPaths(work);

    expect(answers.map(answer => [answer.status, answer.stdout])).toEqual([[0, git(...AUDIT_DIFF)], [0, git(...SCAN_DIFF)]]);
    expect(walk).toEqual(git("ls-files", "-z", "--modified", "--others", "--exclude-standard").split("\0").filter(path => path !== ""));
    const notice = `Git ${version} is older than 2.31, so Rerail's Git calls run the filter programs a repository's own config names, as that Git runs them. Update Git to 2.31 or later.`;
    expect(said.mock.calls.map(([text]) => String(text)).filter(text => text.startsWith("rerail:"))).toEqual([`rerail: ${notice}\n`]);
  }, 60_000);

  it("on Git 2.31 the guard covers each call", async () => {
    const root = await folder();
    vi.stubEnv("PATH", `${await olderGit(root, "2.31.0")}${delimiter}${process.env.PATH ?? ""}`);
    vi.resetModules();
    const { runGit } = await import("../../src/engine/git.js");
    const { work, marker } = await checkoutWithDriver(root);
    const said = vi.spyOn(process.stderr, "write").mockImplementation(() => true);

    expect(runGit(work, AUDIT_DIFF).stdout).toBe("notes.txt\0");
    expect(existsSync(marker)).toBe(false);
    expect(said.mock.calls.map(([text]) => String(text)).filter(text => text.startsWith("rerail:"))).toEqual([]);
  }, 60_000);
});
