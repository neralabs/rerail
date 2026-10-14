import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { ProcessCommandRunner } from "../../../conformance/gates/process.js";
import { buildRerailCorpora, readRerailCorpora } from "../../../conformance/gates/corpus.js";

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
