/**
 * Every upstream file a gated tool-pack's generator read (the `from` of each file in its pulled provenance.json) is in
 * its gates.yaml `watch.tests`, so the nightly upstream report sees every change to the tests Rerail's data comes from.
 * It needs the packs' test data (`npm run fixtures:pull`), which CI's unit job restores.
 */
import assert from "node:assert/strict";
import { join } from "node:path";
import { describe, test } from "vitest";

import { gatedToolPacks, readGatesManifest, type GatesManifest } from "../../../conformance/languages/manifest.js";
import { readFrom, readsUpstream, unwatchedTests } from "./support/watch-coverage.js";

const ROOT = join(import.meta.dirname, "..", "..", "..");

describe("each gated tool-pack's watch.tests covers every upstream file its generator read", () => {
  for (const { pack, tool } of gatedToolPacks(ROOT)) {
    test(`${pack}/${tool}`, () => {
      const from = readFrom(ROOT, pack, tool);
      // A tool-pack whose test data is all Rerail's reads no upstream file (complexity/lizard scores Rerail's corpus).
      if (!readsUpstream(ROOT, pack, tool)) assert.deepEqual(from, []);
      else assert.ok(from.length > 0, `packs/${pack}/upstream/${tool}/provenance.json records no upstream file`);
      assert.deepEqual(unwatchedTests(ROOT, pack, tool, readGatesManifest(ROOT, pack, tool), from), []);
    });
  }
});

/** `manifest` without the `watch.tests` entry `dropped`. */
function without(manifest: GatesManifest, dropped: string): GatesManifest {
  const watch = manifest.watch!;
  assert.ok(watch.tests.includes(dropped), `watch.tests has no ${dropped}`);
  return { ...manifest, watch: { ...watch, tests: watch.tests.filter((entry) => entry !== dropped) } };
}

test("a watch.tests pattern removed leaves the files only it covered unwatched", () => {
  const eslint = readGatesManifest(ROOT, "javascript", "eslint");
  const read = readFrom(ROOT, "javascript", "eslint");
  assert.deepEqual(unwatchedTests(ROOT, "javascript", "eslint", without(eslint, "tests/lib/rules/{rule}.js"), read), read);

  const clippy = readGatesManifest(ROOT, "rust", "clippy");
  assert.deepEqual(unwatchedTests(ROOT, "rust", "clippy", without(clippy, "tests/ui/auxiliary/**"), readFrom(ROOT, "rust", "clippy")), [
    "tests/ui/auxiliary/macro_rules.rs",
    "tests/ui/auxiliary/proc_macro_attr.rs",
    "tests/ui/auxiliary/proc_macro_derive.rs",
    "tests/ui/auxiliary/proc_macros.rs",
  ]);
});
