import assert from "node:assert/strict";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import { readRerailCorpora } from "../../../conformance/port-readiness/corpus.js";

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
