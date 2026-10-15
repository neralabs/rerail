import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, vi } from "vitest";

import { ProcessCommandRunner, type CommandResult, type CommandRunner } from "../../../conformance/gates/process.js";
import { buildCorpus, buildRerailCorpora, corpusCompleteCheck, readRerailCorpora, type PinnedTask } from "../../../conformance/gates/corpus.js";

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

test("a pinned corpus gives matches-real-code every file of the linted type under its paths, sorted, named rerail:<corpus>", async () => {
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
    const built = await buildRerailCorpora([corpus], join(scratch, "cache"), (path) => path.endsWith(".py"), new ProcessCommandRunner());
    assert.deepEqual(built.files.map((file) => [file.task, file.path]), [["rerail:local", "django/b.py"], ["rerail:local", "django/sub/a.py"]]);
    assert.deepEqual(built.tasks.map((task) => [task.task, task.substrate, task.baseSha, task.status, task.files]), [["rerail:local", "rerail", commit, "applied", 2]]);
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
    const downloaded = await buildCorpus(cache, inCorpus, gitRunner("none"), [task({ locator: `https://github.com/o/r/blob/${"b".repeat(40)}/p.patch`, sha256: patchSha })]);
    assert.deepEqual(downloaded.tasks.map((entry) => [entry.status, entry.reason]), [["failed", "downloading its reference patch failed with HTTP 503"]]);

    await write(cache, `corpus/patches/${patchSha}.patch`, PATCH);
    const pinned = [task({ locator: `https://github.com/o/r/blob/${"b".repeat(40)}/p.patch`, sha256: patchSha })];
    const fetchFailed = await buildCorpus(cache, inCorpus, gitRunner("fetch"), pinned);
    assert.deepEqual(fetchFailed.tasks.map((entry) => entry.status), ["failed"]);
    assert.match(fetchFailed.tasks[0]!.reason, /^fetching its base commit failed: fatal: fetch went wrong/u);
    const applyFailed = await buildCorpus(cache, inCorpus, gitRunner("apply"), pinned);
    assert.deepEqual(applyFailed.tasks.map((entry) => entry.status), ["skipped"]);
    assert.match(applyFailed.tasks[0]!.reason, /^its solution does not apply/u);
  } finally {
    vi.unstubAllGlobals();
    await rm(cache, { recursive: true, force: true });
  }
});

test("corpus-complete fails when a recorded file was not fetched, or not with its recorded bytes", () => {
  const recorded = [{ task: "t1", path: "app/x.py", sha256: "s1" }, { task: "t2", path: "app/y.py", sha256: "s2" }];
  const file = (taskId: string, path: string, digest: string) => ({ task: taskId, repository: "o/r", path, source: "", sha256: digest });
  assert.equal(corpusCompleteCheck(recorded, [file("t1", "app/x.py", "s1"), file("t2", "app/y.py", "s2")], []).ok, true);
  const failed = { ...task(null), task: "t2", patchSha256: null, status: "failed" as const, reason: "downloading its reference patch failed with HTTP 429", files: 0 };
  const missing = corpusCompleteCheck(recorded, [file("t1", "app/x.py", "s1")], [failed]);
  assert.equal(missing.ok, false);
  assert.match(missing.detail, /1 of 2 recorded corpus file\(s\) were not fetched .*t2: app\/y\.py.*1 corpus task\(s\) could not be fetched \(t2: downloading its reference patch failed with HTTP 429\)/u);
  assert.equal(corpusCompleteCheck(recorded, [file("t1", "app/x.py", "s1"), file("t2", "app/y.py", "other")], []).ok, false);
});
