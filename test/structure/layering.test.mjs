// Structure gate, layering metrics (#12-#15): each predicate on a tiny in-memory tree.

import { describe, expect, it } from "vitest";
import { memoryRepo } from "../../scripts/check/check-structure/repo.mjs";
import { check as flatDirectories } from "../../scripts/check/check-structure/metrics/12-flat-directory-size.mjs";
import { check as importDirection } from "../../scripts/check/check-structure/metrics/13-import-direction.mjs";
import { check as fixturesRoot } from "../../scripts/check/check-structure/metrics/14-fixtures-root.mjs";
import { check as duplicates } from "../../scripts/check/check-structure/metrics/15-duplicate-concepts.mjs";

const rule = (id) => JSON.stringify({ format: "rerail.policy/v1", id, clauses: [] });
const paths = (result) => result.findings.map((item) => item.path);

function many(prefix, count, extension) {
  return Object.fromEntries(Array.from({ length: count }, (_, i) => [`${prefix}/f${i}${extension}`, ""]));
}

describe("#12 flat directory size", () => {
  it("allows 25 files and flags 26 in a code directory", () => {
    const result = flatDirectories(memoryRepo({ ...many("src/engine", 25, ".ts"), ...many("src", 26, ".ts") }));
    expect(result.findings).toEqual([{ path: "src/", message: "26 files directly in this directory (limit 25)" }]);
  });

  it("counts data files in a code directory, and exempts pack data and fixtures", () => {
    const result = flatDirectories(
      memoryRepo({
        ...many("test", 20, ".json"),
        "test/a.test.ts": "",
        ...many("test/more", 6, ".json"),
        ...many("packs/tslint/rules", 80, ".json"),
        ...many("packs/tslint/upstream/tslint/fixtures/x", 40, ".ts"),
        ...many("fixtures/php/scope", 40, ".php"),
      }),
    );
    expect(paths(result)).toEqual([]);
    const crowded = flatDirectories(memoryRepo({ ...many("test", 25, ".json"), "test/a.test.ts": "" }));
    expect(paths(crowded)).toEqual(["test/"]);
  });
});

describe("#13 import direction (fallback scan)", () => {
  it("flags engine→host, host→host, src outward, and outside imports of src/ internals", async () => {
    const result = await importDirection(
      memoryRepo({
        "src/engine/a.ts": 'import { h } from "../hosts/omp/h.js";\nimport { b } from "./b.js";',
        "src/engine/b.ts": "",
        "src/engine/index.ts": 'export * from "./b.js";',
        "src/hosts/omp/h.ts": 'import { p } from "../pi/p.js";\nimport { b } from "../../engine/index.js";',
        "src/hosts/pi/p.ts": 'import pkg from "../../../package.json" with { type: "json" };',
        "package.json": "{}",
        "integration/x.test.ts": 'import { b } from "../src/engine/b.js";\nimport { e } from "../src/engine/index.js";',
        "test/engine/b.test.ts": 'import { b } from "../../src/engine/b.js";',
      }),
    );
    expect(result.findings.map((item) => `${item.path}:${item.line} ${item.message.split("]")[0]}]`)).toEqual([
      "integration/x.test.ts:1 [public-api]",
      "src/engine/a.ts:1 [engine-to-host]",
      "src/hosts/omp/h.ts:1 [host-to-host]",
      "src/hosts/pi/p.ts:1 [src-outward]",
    ]);
  });
});

describe("#14 fixtures root", () => {
  it("allows one resolver per language and flags a second", () => {
    const resolver = 'const root = resolve(repositoryRoot, process.env.RERAIL_FIXTURES_DIR || "fixtures");';
    const one = fixturesRoot(memoryRepo({ "test/support/oracles.ts": resolver, "native/build.rs": 'let f = root.join("fixtures"); // RERAIL_FIXTURES_DIR' }));
    expect(one.findings).toEqual([]);
    expect(one.actual).toBe("0; Rust 1, TypeScript 1");
    const two = fixturesRoot(
      memoryRepo({
        "test/support/oracles.ts": resolver,
        "test/engine/a.test.ts": 'const DIR = fileURLToPath(new URL("../../fixtures/php/", import.meta.url));',
      }),
    );
    expect(paths(two)).toEqual(["test/engine/a.test.ts", "test/support/oracles.ts"]);
  });

  it("flags production code that resolves fixtures/", () => {
    const result = fixturesRoot(memoryRepo({ "src/engine/x.ts": 'const p = join(root, "fixtures", "php");' }));
    expect(result.findings[0]).toEqual({ path: "src/engine/x.ts", line: 1, message: "production code resolves fixtures/" });
  });

  it("ignores directory-name lists, comparisons, and nested upstream fixtures/ dirs", () => {
    const result = fixturesRoot(
      memoryRepo({
        "src/matcher.ts": 'const SKIP = new Set(["fixtures", "__fixtures__"]);\nif (path.startsWith("fixtures/") || x) {}',
        "scripts/build-probes.mjs": 'const dir = join(ROOT, "labels", "fixtures");',
        "native/src/oracle_tests.rs": 'read(oracle_root().join("fixtures").join(name))',
      }),
    );
    expect(result.findings).toEqual([]);
  });
});

describe("#15 duplicate concepts", () => {
  it("allows option variants under candidates/<rule>/ and flags a rule id defined twice otherwise", () => {
    const result = duplicates(
      memoryRepo({
        "packs/tslint/rules/quotemark.json": rule("tslint:quotemark"),
        "packs/tslint/candidates/quotemark/single.json": rule("tslint:quotemark"),
        "packs/tslint/candidates/quotemark/jsx/double.json": rule("tslint:quotemark"),
        "packs/tslint/rules/typedef.json": rule("tslint:typedef"),
        "packs/tslint/candidates/typedef.json": rule("tslint:typedef"),
        "packs/php/rules/a.json": rule("shared:a"),
        "packs/python/rules/a.json": rule("shared:a"),
      }),
    );
    expect(paths(result)).toEqual([
      "packs/php/rules/a.json",
      "packs/python/rules/a.json",
      "packs/tslint/candidates/typedef.json",
      "packs/tslint/rules/typedef.json",
    ]);
  });

  it("reports the conformance.json files whose format differs from the common one", () => {
    const runner = JSON.stringify({ pack: "x", family: "x", rules: [] });
    const result = duplicates(
      memoryRepo({
        "packs/php/conformance.json": runner,
        "packs/python/conformance.json": runner,
        "packs/tslint/conformance.json": JSON.stringify({ format: "rerail-tslint-golden/v1", rules: [] }),
        "fixtures/packs/broken/conformance.json": JSON.stringify({ format: "nonsense" }),
      }),
    );
    expect(result.findings).toEqual([
      { path: "packs/tslint/conformance.json", message: "[conformance-format] format rerail-tslint-golden/v1; the others use untagged" },
    ]);
  });

  it("flags shared code importing a pack-specific harness, not the pack's own code", () => {
    const result = duplicates(
      memoryRepo({
        "packs/tslint/pack.json": "{}",
        "test/conformance/tslint/runner.ts": "",
        "test/conformance/tslint/golden.test.ts": 'import { run } from "./runner.js";',
        "test/conformance/languages/runner.ts": 'import { run } from "../tslint/runner.js";',
      }),
    );
    expect(result.findings).toEqual([
      {
        path: "test/conformance/languages/runner.ts",
        line: 1,
        message: "[harness-import] imports test/conformance/tslint/, a pack-specific harness",
      },
    ]);
  });
});
