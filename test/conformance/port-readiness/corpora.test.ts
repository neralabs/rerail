import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { test, vi } from "vitest";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { ProcessCommandRunner, type CommandOptions, type CommandResult, type CommandRunner } from "../../../conformance/port-readiness/process.js";
import { buildCorpus, buildRerailCorpora, corpusCompleteCheck, pruneCheckouts, readRerailCorpora, wantedPlaces, type PinnedTask } from "../../../conformance/port-readiness/corpus.js";
import { upstreamCheckoutDirectory } from "../../../conformance/port-readiness/recorded-adapter.js";
import { readRecordingLock } from "../../../conformance/port-readiness/recording/store.js";

const PACK = "python";
const DJANGO = { name: "django", repository: "https://github.com/django/django", commit: "f".repeat(40), paths: ["django"], exclude: [] };

async function write(root: string, path: string, text: string): Promise<void> {
  await mkdir(join(root, path, ".."), { recursive: true });
  await writeFile(join(root, path), text);
}

test("a rerail pack's corpora are read from its conformance manifests, by name", async () => {
  const checkout = await mkdtemp(join(tmpdir(), "validation-rerail-corpora-"));
  try {
    assert.deepEqual(await readRerailCorpora(checkout, PACK), []);
    const zeta = { ...DJANGO, name: "zeta", paths: ["b", "a"], exclude: ["**/z.py"] };
    await write(checkout, "packs/python/conformance.json", JSON.stringify({ pack: "python", corpora: [zeta, DJANGO] }));
    await write(checkout, "packs/python/candidates/conformance.json", JSON.stringify({ corpora: [DJANGO] }));
    assert.deepEqual(await readRerailCorpora(checkout, PACK), [DJANGO, { ...zeta, paths: ["a", "b"] }]);
    // One name may not pin two different corpora, and a path may not climb out of the repository.
    await write(checkout, "packs/python/candidates/conformance.json", JSON.stringify({ corpora: [{ ...DJANGO, commit: "e".repeat(40) }] }));
    await assert.rejects(readRerailCorpora(checkout, PACK), /pins two different corpora named django/u);
    await write(checkout, "packs/python/candidates/conformance.json", JSON.stringify({ corpora: [{ ...DJANGO, paths: ["../x"] }] }));
    await assert.rejects(readRerailCorpora(checkout, PACK), /pins a corpus the gates cannot read/u);
  } finally {
    await rm(checkout, { recursive: true, force: true });
  }
});

test("a pinned corpus gives the files goldens name under its paths, of the linted type and not excluded, sorted, named rerail:<corpus>", async () => {
  const scratch = await mkdtemp(join(tmpdir(), "validation-rerail-corpora-"));
  try {
    const source = join(scratch, "upstream");
    await write(source, "django/b.py", "b = 1\n");
    await write(source, "django/sub/a.py", "a = 1\n");
    await write(source, "django/sub/skip.py", "skip = 1\n");
    await write(source, "django/notes.txt", "not python\n");
    await write(source, "other/c.py", "c = 1\n");
    const git = (...args: string[]) => execFileSync("git", ["-C", source, ...args], { encoding: "utf8" }).trim();
    git("init", "-q");
    git("add", ".");
    git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "corpus");
    const commit = git("rev-parse", "HEAD");
    execFileSync("git", ["clone", "-q", "--bare", source, `${source}.git`]);
    const corpus = { name: "local", repository: source, commit, paths: ["django"], exclude: ["**/skip.py"] };
    const inCorpus = (path: string) => path.endsWith(".py");
    // Goldens name two linted files, an excluded one, a file of another type, one outside the pinned paths and one in
    // another corpus: only the first two are read.
    const wanted = wantedPlaces(["django/b.py", "django/sub/a.py", "django/sub/skip.py", "django/notes.txt", "other/c.py"].map((path) => ({ task: "rerail:local", path })).concat([{ task: "rerail:elsewhere", path: "django/b.py" }]));
    const built = await buildRerailCorpora([corpus], join(scratch, "cache"), inCorpus, new ProcessCommandRunner(), wanted);
    assert.deepEqual(built.files.map((file) => [file.task, file.path]), [["rerail:local", "django/b.py"], ["rerail:local", "django/sub/a.py"]]);
    assert.deepEqual(built.tasks.map((task) => [task.task, task.substrate, task.baseSha, task.status, task.files]), [["rerail:local", "rerail", commit, "applied", 2]]);
    // The checkout holds only the named files under the pinned paths.
    const checkout = join(scratch, "cache", "corpus", "rerail", "local", commit);
    assert.deepEqual(["django/b.py", "django/sub/a.py", "django/sub/skip.py", "django/notes.txt", "other/c.py"].map((path) => existsSync(join(checkout, path))), [true, true, true, true, false]);
    // A corpus no golden names is not fetched.
    const none = await buildRerailCorpora([{ ...corpus, name: "unnamed" }], join(scratch, "cache"), inCorpus, new ProcessCommandRunner(), wanted);
    assert.deepEqual(none.tasks.map((task) => [task.task, task.status, task.reason]), [["rerail:unnamed", "skipped", "no golden names a file of it"]]);
    assert.equal(existsSync(join(scratch, "cache", "corpus", "rerail", "unnamed")), false);
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
});

const PATCH = "diff --git a/app/x.py b/app/x.py\n--- a/app/x.py\n+++ b/app/x.py\n@@ -1 +1 @@\n-a = 1\n+a = 2\n";
const patchSha = createHash("sha256").update(PATCH).digest("hex");
const task = (reference: PinnedTask["reference"]): PinnedTask => ({ task: "t1", substrate: "swe-atlas", repository: "o/r", baseSha: "a".repeat(40), reference });

/** Answers every git command, failing the one named `failing`. */
function gitRunner(failing: string): CommandRunner {
  return {
    async run(argv): Promise<CommandResult> {
      return argv.includes(failing) ? { exitCode: 128, stdout: "", stderr: `fatal: ${failing} went wrong` } : { exitCode: 0, stdout: "", stderr: "" };
    },
  };
}

test("a failed download or git fetch is a failed task, unlike a solution that does not apply", async () => {
  const cache = await mkdtemp(join(tmpdir(), "validation-corpus-"));
  try {
    const inCorpus = (path: string) => path.endsWith(".py");
    vi.stubGlobal("fetch", async () => new Response("", { status: 503 }));
    const wanted = wantedPlaces([{ task: "t1", path: "app/x.py" }]);
    const downloaded = await buildCorpus(cache, inCorpus, gitRunner("none"), wanted, [task({ locator: `https://github.com/o/r/blob/${"b".repeat(40)}/p.patch`, sha256: patchSha })]);
    assert.deepEqual(downloaded.tasks.map((entry) => [entry.status, entry.reason]), [["failed", "downloading its reference patch failed with HTTP 503"]]);

    await write(cache, `corpus/patches/${patchSha}.patch`, PATCH);
    const pinned = [task({ locator: `https://github.com/o/r/blob/${"b".repeat(40)}/p.patch`, sha256: patchSha })];
    const fetchFailed = await buildCorpus(cache, inCorpus, gitRunner("fetch"), wanted, pinned);
    assert.deepEqual(fetchFailed.tasks.map((entry) => entry.status), ["failed"]);
    assert.match(fetchFailed.tasks[0]!.reason, /^fetching its base commit failed: fatal: fetch went wrong/u);
    const applyFailed = await buildCorpus(cache, inCorpus, gitRunner("apply"), wanted, pinned);
    assert.deepEqual(applyFailed.tasks.map((entry) => entry.status), ["skipped"]);
    assert.match(applyFailed.tasks[0]!.reason, /^its solution does not apply/u);
    // A task no golden names is neither downloaded nor fetched, and one whose named files its solution does not change reads none.
    const unnamed = await buildCorpus(cache, inCorpus, gitRunner("fetch"), wantedPlaces([{ task: "t2", path: "app/x.py" }]), pinned);
    assert.deepEqual(unnamed.tasks.map((entry) => [entry.status, entry.reason]), [["skipped", "no golden names a file of it"]]);
    const otherFile = await buildCorpus(cache, inCorpus, gitRunner("fetch"), wantedPlaces([{ task: "t1", path: "app/other.py" }]), pinned);
    assert.deepEqual(otherFile.tasks.map((entry) => [entry.status, entry.reason]), [["skipped", "its solution changes no file a golden names"]]);
  } finally {
    vi.unstubAllGlobals();
    await rm(cache, { recursive: true, force: true });
  }
});

test("corpus-complete fails when a task or corpus a golden names failed to fetch, and only then", () => {
  const goldens = [{ task: "t1" }, { task: "t2" }];
  const status = (taskId: string, value: "applied" | "skipped" | "failed") => ({ ...task(null), task: taskId, patchSha256: null, status: value, reason: value === "failed" ? "downloading its reference patch failed with HTTP 429" : "", files: 0 });
  assert.equal(corpusCompleteCheck(goldens, [status("t1", "applied"), status("t2", "applied"), status("t3", "failed")]).ok, true);
  const failed = corpusCompleteCheck(goldens, [status("t1", "applied"), status("t2", "failed")]);
  assert.equal(failed.ok, false);
  assert.match(failed.detail, /^1 corpus task\(s\) could not be fetched \(t2: downloading its reference patch failed with HTTP 429\): run the gates again$/u);
  // A task that applied but lacks a golden's file, or has it with other bytes, fails that golden, not the tool-pack.
  assert.equal(corpusCompleteCheck(goldens, [status("t1", "applied"), status("t2", "skipped")]).ok, true);
});

/** A ProcessCommandRunner that also records every command it runs. */
class RecordingRunner extends ProcessCommandRunner {
  readonly calls: string[] = [];
  override async run(argv: readonly string[], options?: CommandOptions): Promise<CommandResult> {
    this.calls.push(argv.join(" "));
    return await super.run(argv, options);
  }
}

test("a pinned corpus is fetched blobless, and a golden on another of its files later fetches only that file", async () => {
  const scratch = await mkdtemp(join(tmpdir(), "validation-rerail-corpora-"));
  try {
    const source = join(scratch, "upstream");
    for (const name of ["a", "b", "c"]) await write(source, `django/${name}.py`, `${name} = 1\n`);
    const git = (...args: string[]) => execFileSync("git", ["-C", source, ...args], { encoding: "utf8" }).trim();
    git("init", "-q");
    git("add", ".");
    git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "corpus");
    const commit = git("rev-parse", "HEAD");
    execFileSync("git", ["clone", "-q", "--bare", source, `${source}.git`]);
    // As GitHub serves it: a server that ignores the filter sends every blob anyway, which would prove nothing.
    execFileSync("git", ["-C", `${source}.git`, "config", "uploadpack.allowFilter", "true"]);
    const corpus = { name: "local", repository: source, commit, paths: ["django"], exclude: [] };
    const inCorpus = (path: string) => path.endsWith(".py");
    const checkout = join(scratch, "cache", "corpus", "rerail", "local", commit);
    // The blobs the checkout's commit names that it does not hold.
    const missing = () => execFileSync("git", ["-C", checkout, "rev-list", "--objects", "--missing=print", "HEAD"], { encoding: "utf8" }).split("\n").filter((line) => line.startsWith("?")).length;
    const runner = new RecordingRunner();
    const first = await buildRerailCorpora([corpus], join(scratch, "cache"), inCorpus, runner, wantedPlaces([{ task: "rerail:local", path: "django/a.py" }]));
    assert.deepEqual(first.files.map((file) => file.path), ["django/a.py"]);
    assert.ok(runner.calls.includes(`git -C ${checkout} fetch -q --depth 1 --filter=blob:none origin ${commit}`), runner.calls.join("\n"));
    assert.equal(missing(), 2);
    // The cached checkout's sparse set is widened, not fetched again: git fetches the one blob it now needs.
    const wider = await buildRerailCorpora([corpus], join(scratch, "cache"), inCorpus, runner, wantedPlaces(["django/a.py", "django/b.py"].map((path) => ({ task: "rerail:local", path }))));
    assert.deepEqual(wider.files.map((file) => [file.path, file.source]), [["django/a.py", "a = 1\n"], ["django/b.py", "b = 1\n"]]);
    assert.equal(missing(), 1);
    assert.equal(runner.calls.filter((call) => call.includes(" fetch ")).length, 1);
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
});

test("pruning removes only the other commits beside each pinned checkout, and only under the cache root's checkout directories", async () => {
  const cache = await mkdtemp(join(tmpdir(), "validation-corpus-prune-"));
  const outside = await mkdtemp(join(tmpdir(), "validation-corpus-prune-outside-"));
  try {
    const [pinned, earlier, other] = ["a", "b", "c"].map((digit) => digit.repeat(40)) as [string, string, string];
    const directories = {
      corpus: join(cache, "corpus", "rerail", "django", pinned),
      corpusEarlier: join(cache, "corpus", "rerail", "django", earlier),
      upstream: join(cache, "upstream", "bandit", pinned),
      upstreamEarlier: join(cache, "upstream", "bandit", other),
      // A corpus or repository no kept checkout names may be another tool-pack's, or the recorder's.
      unnamedCorpus: join(cache, "corpus", "rerail", "flask", earlier),
      unnamedUpstream: join(cache, "upstream", "ruff", earlier),
      // Only a directory named for a commit is a checkout.
      notCommit: join(cache, "corpus", "rerail", "django", "notes"),
      taskCheckout: join(cache, "corpus", "checkouts", `t1-${"0".repeat(12)}`),
      results: join(cache, "results", "python", "bandit", earlier),
    };
    for (const directory of Object.values(directories)) await write(directory, "x.py", "x = 1\n");
    await write(join(cache, "upstream", "bandit"), "d".repeat(40), "a file named like a commit\n");
    // A link named like a commit is not followed out of the cache, nor removed.
    await write(outside, `${earlier}/x.py`, "x = 1\n");
    await symlink(outside, join(cache, "upstream", "bandit", "e".repeat(40)));
    const removed = await pruneCheckouts(cache, [directories.corpus, directories.upstream, join(cache, "corpus", "rerail", "vscode", pinned)]);
    assert.deepEqual(removed, [directories.corpusEarlier, directories.upstreamEarlier]);
    for (const [name, directory] of Object.entries(directories)) assert.equal(existsSync(directory), name !== "corpusEarlier" && name !== "upstreamEarlier", name);
    for (const digit of ["d", "e"]) assert.ok(existsSync(join(cache, "upstream", "bandit", digit.repeat(40))), digit);
    assert.ok(existsSync(join(outside, earlier)));
    assert.equal(existsSync(join(cache, "corpus", "rerail", "vscode")), false);
    // A kept checkout anywhere else is refused before anything is removed.
    await write(directories.upstreamEarlier, "x.py", "x = 1\n");
    for (const elsewhere of [join(cache, "results", "python", pinned), join(cache, "corpus", "checkouts", pinned), join(cache, "..", "upstream", "bandit", pinned), join(cache, "upstream", pinned)]) {
      await assert.rejects(pruneCheckouts(cache, [directories.upstream, elsewhere]), /is not a checkout under/u);
    }
    // So is one whose directory is a link out of the cache.
    await symlink(outside, join(cache, "upstream", "linked"));
    await assert.rejects(pruneCheckouts(cache, [directories.upstream, join(cache, "upstream", "linked", pinned)]), /leads out of the cache root/u);
    assert.ok(existsSync(directories.upstreamEarlier));
    assert.ok(existsSync(join(outside, earlier)));
  } finally {
    await rm(cache, { recursive: true, force: true });
    await rm(outside, { recursive: true, force: true });
  }
});

test("gates prune keeps a tool-pack's pinned corpora and upstream checkout and removes their earlier commits, logging each", async () => {
  const cache = await mkdtemp(join(tmpdir(), "validation-corpus-prune-"));
  try {
    const [corpus] = await readRerailCorpora(PACKAGE_ROOT, PACK);
    const { pins } = readRecordingLock(PACKAGE_ROOT, PACK, "bandit");
    const earlier = "0".repeat(40);
    const kept = [join(cache, "corpus", "rerail", corpus!.name, corpus!.commit), upstreamCheckoutDirectory(cache, pins)];
    const stale = [join(cache, "corpus", "rerail", corpus!.name, earlier), upstreamCheckoutDirectory(cache, { ...pins, commit: earlier })];
    for (const directory of [...kept, ...stale]) await write(directory, "x.py", "x = 1\n");
    const stdout = execFileSync(process.execPath, ["--import", "tsx", join(PACKAGE_ROOT, "conformance", "port-readiness", "cli.ts"), "prune", "--pack", PACK, "--tool", "bandit", "--cache", cache], { cwd: PACKAGE_ROOT, encoding: "utf8" });
    assert.deepEqual(stdout.trim().split("\n"), [
      ...stale.map((directory) => `removed ${relative(cache, directory)}: no longer pinned by python/bandit`),
      `pruned 2 checkout(s) no longer pinned from ${cache}`,
    ]);
    assert.deepEqual([...kept, ...stale].map((directory) => existsSync(directory)), [true, true, false, false]);
  } finally {
    await rm(cache, { recursive: true, force: true });
  }
});
