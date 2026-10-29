/**
 * Whether a gated tool-pack's `watch.tests` (gates.yaml) covers every upstream file its generator read, as the pulled
 * test data's provenance.json records them (each copied or extracted file's `from`).
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { toolPackDirectory, type GatesManifest } from "../../../../conformance/languages/manifest.js";
import { expandWatch, watchMatcher } from "../../../../conformance/languages/watch.js";
import { requirePackFixture } from "../../../../scripts/s3/pack-fixtures.mjs";

/** The upstream paths the tool-pack's pulled provenance.json records its files were read from, byte-sorted. */
export function readFrom(root: string, pack: string, tool: string): string[] {
  const path = requirePackFixture(join(toolPackDirectory(root, pack, tool), "provenance.json"));
  const provenance = JSON.parse(readFileSync(path, "utf8")) as { files: Record<string, { from?: string }> };
  const from = Object.values(provenance.files).flatMap((file) => (file.from === undefined ? [] : [file.from]));
  return [...new Set(from)].sort((left, right) => Buffer.compare(Buffer.from(left), Buffer.from(right)));
}

/**
 * The paths of `from` that `manifest`'s expanded `watch.tests` does not cover; throws when it has no watch. Which files
 * a `{ named-by }` test names is known only from upstream's text, which this check does not read, so a path anywhere
 * under the top directory of a named-by test (`tests/` for `tests/PHPStan/Rules/X/YTest.php`, whose files climb out
 * of its own directory with `__DIR__ . '/../…'`) counts as covered; the nightly report watches exactly the named ones.
 */
export function unwatchedTests(root: string, pack: string, tool: string, manifest: GatesManifest, from: readonly string[]): string[] {
  const watch = expandWatch(root, pack, tool, manifest);
  if (watch === undefined) throw new Error(`packs/${pack}/upstream/${tool}/gates.yaml has no watch block`);
  const watched = watchMatcher(watch.tests);
  const trees = Object.keys(watch.namedTests).map((test) => `${test.split("/")[0]!}/`);
  return from.filter((path) => !watched(path) && !trees.some((tree) => path.startsWith(tree)));
}
