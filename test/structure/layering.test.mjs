// Structure gate, layering metrics (#12-#15): each predicate on a tiny in-memory tree.

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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
        ...many("packs/example/rules", 80, ".json"),
        ...many("packs/example/upstream/tool/fixtures/x", 40, ".ts"),
        ...many("fixtures/php/scope", 40, ".php"),
      }),
    );
    expect(paths(result)).toEqual([]);
    const crowded = flatDirectories(memoryRepo({ ...many("test", 25, ".json"), "test/a.test.ts": "" }));
    expect(paths(crowded)).toEqual(["test/"]);
  });
});

describe("#13 import direction", () => {
  it("is the boundary lint's result over the tracked files", async () => {
    const root = mkdtempSync(join(tmpdir(), "rerail-import-direction-"));
    try {
      mkdirSync(join(root, "scripts/check"), { recursive: true });
      const lint = `export function checkBoundaries(root, { files }) {
  return { violations: files.filter((file) => file.startsWith("src/")).map((file) => ({ file, line: 2, rule: "layer-order", message: "x" })) };
}`;
      writeFileSync(join(root, "scripts/check/check-boundaries.mjs"), lint);
      const result = await importDirection(
        memoryRepo({ "scripts/check/check-boundaries.mjs": lint, "src/a.ts": "", "src/b.ts": "", "test/c.ts": "" }, root),
      );
      expect(result.actual).toBe("2 in 2 files");
      expect(result.findings).toEqual([
        { path: "src/a.ts", line: 2, message: "[layer-order] x" },
        { path: "src/b.ts", line: 2, message: "[layer-order] x" },
      ]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("fails without the boundary lint", async () => {
    const result = await importDirection(memoryRepo({ "src/engine/a.ts": 'import { h } from "../hosts/omp/h.js";' }));
    expect(result.findings).toEqual([
      { path: "scripts/check/check-boundaries.mjs", message: "missing: the layer boundary lint this metric runs" },
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

  it.each([
    ["dot segments before it", 'const p = join(here, "..", "..", "fixtures", "php");', 1],
    ["a call as the root", 'const p = resolve(process.cwd(), "fixtures");', 1],
    ["a qualified join with a call root", 'const p = path.join(repoRoot(), "fixtures");', 1],
    ["a call split over lines", 'const p = join(\n  root,\n  "fixtures",\n);', 3],
    ["a root holding a string with parens and commas", 'const p = join(pick("a,(b"), ".", "fixtures");', 1],
  ])("finds the bare segment first after a root: %s", (_, source, line) => {
    expect(fixturesRoot(memoryRepo({ "src/engine/x.ts": source })).findings).toEqual([
      { path: "src/engine/x.ts", line, message: "production code resolves fixtures/" },
    ]);
  });

  it("ignores directory-name lists, comparisons, and nested upstream fixtures/ dirs", () => {
    const result = fixturesRoot(
      memoryRepo({
        "src/matcher.ts": 'const SKIP = new Set(["fixtures", "__fixtures__"]);\nif (path.startsWith("fixtures/") || x) {}',
        "scripts/build-probes.mjs": 'const dir = join(ROOT, "labels", "fixtures");\nconst up = join(\n  ROOT,\n  "labels",\n  "fixtures",\n);',
        "scripts/names.mjs": 'const dirs = join(root, ["fixtures"]);\nconst n = join(a, b) + f("fixtures");',
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
        "packs/example/rules/quotemark.json": rule("example:quotemark"),
        "packs/example/candidates/quotemark/single.json": rule("example:quotemark"),
        "packs/example/candidates/quotemark/jsx/double.json": rule("example:quotemark"),
        "packs/example/rules/typedef.json": rule("example:typedef"),
        "packs/example/candidates/typedef.json": rule("example:typedef"),
        "packs/php/rules/a.json": rule("shared:a"),
        "packs/python/rules/a.json": rule("shared:a"),
      }),
    );
    expect(paths(result)).toEqual([
      "packs/example/candidates/typedef.json",
      "packs/example/rules/typedef.json",
      "packs/php/rules/a.json",
      "packs/python/rules/a.json",
    ]);
  });

  it("reports the conformance.json files whose format differs from the common one", () => {
    const runner = JSON.stringify({ pack: "x", family: "x", rules: [] });
    const result = duplicates(
      memoryRepo({
        "packs/php/conformance.json": runner,
        "packs/python/conformance.json": runner,
        "packs/example/conformance.json": JSON.stringify({ format: "rerail-example-golden/v1", rules: [] }),
        "fixtures/packs/broken/conformance.json": JSON.stringify({ format: "nonsense" }),
      }),
    );
    expect(result.findings).toEqual([
      { path: "packs/example/conformance.json", message: "[conformance-format] format rerail-example-golden/v1; the others use untagged" },
    ]);
  });

  it("flags shared code importing a pack-specific harness, not the pack's own code", () => {
    const result = duplicates(
      memoryRepo({
        "packs/example/pack.json": "{}",
        "test/conformance/example/runner.ts": "",
        "test/conformance/example/golden.test.ts": 'import { run } from "./runner.js";',
        "test/conformance/languages/runner.ts": 'import { run } from "../example/runner.js";',
      }),
    );
    expect(result.findings).toEqual([
      {
        path: "test/conformance/languages/runner.ts",
        line: 1,
        message: "[harness-import] imports test/conformance/example/, a pack-specific harness",
      },
    ]);
  });
});
