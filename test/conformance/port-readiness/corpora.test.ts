import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, vi } from "vitest";

import { ProcessCommandRunner, type CommandResult, type CommandRunner } from "../../../conformance/port-readiness/process.js";
import { buildCorpus, buildRerailCorpora, corpusCompleteCheck, readRerailCorpora, wantedPlaces, type PinnedTask } from "../../../conformance/port-readiness/corpus.js";

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
