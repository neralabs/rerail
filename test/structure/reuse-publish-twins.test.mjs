// REUSE.toml: a tool-pack's publish/ (scripts/s3/pack-fixtures.mjs, PUBLISH_DIRECTORY) holds pack test data in its
// pulled layout, tracked while a pull request publishes it, so every annotation of a pack test data path carries the
// same path under publish/ and `reuse lint` licenses the tracked copy as it licenses the pulled one.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parseReuseToml } from "../../scripts/check/check-structure/metrics/07-pack-licence-annotations.mjs";
import { isPackFixture } from "../../scripts/s3/pack-fixtures.mjs";

const REUSE = readFileSync(new URL("../../REUSE.toml", import.meta.url), "utf8");
/** A tool-pack path outside publish/, and its publish/ twin's parts. */
const DATA = /^packs\/([^/]+)\/upstream\/([^/]+)\/(?!publish\/)(.+)$/u;

describe("REUSE.toml", () => {
  it("annotates the publish/ twin of every pack test data path in the same annotation", () => {
    const annotations = parseReuseToml(REUSE);
    const data = annotations.flatMap((annotation) => annotation.paths.filter((path) => DATA.test(path) && isPackFixture(path)).map((path) => ({ annotation, path })));
    expect(data.length).toBeGreaterThan(0);
    const missing = data
      .filter(({ annotation, path }) => !annotation.paths.includes(path.replace(DATA, "packs/$1/upstream/$2/publish/$3")))
      .map(({ path }) => path);
    expect(missing).toEqual([]);
  });
});
