// scripts/check/ci/test-selection.mjs: which unit test files a change can affect, and the shards that run them.
import ts from "typescript";
import { describe, expect, it } from "vitest";
import { affected, namedPaths, names, outputs, seen, shards, stringLiterals } from "../../../scripts/check/ci/test-selection.mjs";

const literals = (text) => stringLiterals(ts, "x.ts", text);
const named = (file, text, top = ["packs", "native", "src", "test", ".rerail"]) => namedPaths(file, literals(text), new Set(top));

describe("the paths a file names", () => {
  it("are its string literals, less the modules it imports", () => {
    const text = [
      'import { a } from "./a.js";',
      'export * from "./b.js";',
      'const c = await import("./c.js");',
      'vi.mock("./d.js");',
      'const p = join(root, "packs", `python/${name}`);',
    ].join("\n");
    expect(literals(text)).toEqual(["packs", "python/", ""]);
  });

  it("read from the root and, for ./ and ../, from the file's directory; one word only as a top-level entry", () => {
    const found = named("test/a/x.test.ts", 'f("packs"); f("src"); f("./data/in.json"); f("../../fixtures/host/*.json"); f("test"); f("no such word"); f("mode")');
    expect([...found.paths].sort()).toEqual(["data/in.json", "fixtures/host", "packs", "test/a/data/in.json"]);
    expect([...found.names]).toEqual(["in.json"]);
  });

  it("hold a changed file when they name it, a directory above it, or its file name", () => {
    const found = named("src/x.ts", 'f("packs"); f("bandit.expected.json")');
    expect(names(found, "packs/python/rules/a.rerail")).toBe(true);
    expect(names(found, "fixtures/host/lint-config/bandit/bandit.expected.json")).toBe(true);
    expect(names(found, "fixtures/host/other.json")).toBe(false);
  });
});

describe("what the tests see of a change", () => {
  it("is the addon for the engine's inputs and dist/ for src/", () => {
    expect(seen(["native/src/lib.rs", "src/cli/cli.ts"]).sort()).toEqual(["dist/cli/cli.js", "native/rerail_engine.node", "native/src/lib.rs", "src/cli/cli.ts"]);
    expect(seen(["Cargo.lock"])).toContain("native/rerail_engine.node");
  });
});

describe("affected", () => {
  // a imports lib.ts; b names "native" (the engine's loader); c spawns scripts/run.mjs, which imports helper.mjs.
  const closure = new Map([
    ["test/a.test.ts", new Set(["test/a.test.ts", "src/lib.ts"])],
    ["test/b.test.ts", new Set(["test/b.test.ts", "src/engine/native.ts"])],
    ["test/c.test.ts", new Set(["test/c.test.ts", "scripts/run.mjs", "scripts/helper.mjs"])],
  ]);
  const texts = { "src/engine/native.ts": 'join(root, "native", file)' };
  const graph = { closure, named: (file) => named(file, texts[file] ?? "") };
  const tests = [...closure.keys()];

  it("selects the tests whose closure holds the change, or names it", () => {
    expect(affected(tests, graph, ["src/lib.ts"])).toEqual(["test/a.test.ts"]);
    expect(affected(tests, graph, ["native/src/x.rs"])).toEqual(["test/b.test.ts"]);
    expect(affected(tests, graph, ["accel/src/y.rs"])).toEqual(["test/b.test.ts"]);
    expect(affected(tests, graph, ["scripts/helper.mjs"])).toEqual(["test/c.test.ts"]);
    expect(affected(tests, graph, ["docs/readme.md"])).toEqual([]);
  });
});

describe("shards and outputs", () => {
  it("run as many shards as the selection's share needs, at least one", () => {
    expect(shards(449, 449, 5)).toEqual([1, 2, 3, 4, 5]);
    expect(shards(100, 449, 5)).toEqual([1, 2]);
    expect(shards(0, 449, 5)).toEqual([1]);
  });

  it("run no unit shard for none, and Windows' first shard when its hook test is selected", () => {
    expect(outputs({ unit: "none", tests: [], all: 449, windowsExtra: true })).toEqual({ unit: "none", "unit-tests": "", "unit-shards": "[]", "windows-shards": "[1]" });
    expect(outputs({ unit: "none", tests: [], all: 449, windowsExtra: false })["windows-shards"]).toBe("[]");
    expect(outputs({ unit: "some", tests: ["test/a.test.ts"], all: 449, windowsExtra: false })).toEqual({ unit: "some", "unit-tests": "test/a.test.ts", "unit-shards": "[1]", "windows-shards": "[1]" });
    expect(outputs({ unit: "all", tests: [], all: 449, windowsExtra: true })["unit-shards"]).toBe("[1,2]");
  });
});
