// scripts/check/ci/test-selection.mjs: which unit test files a change can affect, and the shards that run them.
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";
import { engineSources, isEngineInput } from "../../../scripts/check/ci/engine-inputs.mjs";
import { affected, namedPaths, names, outputs, seen, select, shards, stringLiterals } from "../../../scripts/check/ci/test-selection.mjs";

const ROOT = fileURLToPath(new URL("../../..", import.meta.url));

const literals = (text) => stringLiterals(ts, "x.ts", text);
const named = (file, text, top = ["packs", "native", "src", "test", ".rerail"]) => namedPaths(file, literals(text), new Set(top));
// The engine's inputs as engine-inputs.mjs's `engineSources` gives them: its crates' directories and the files it embeds.
const SOURCES = { dirs: ["native/", "accel/", "forest/", "languages/", "tools/bitlex/"], paths: new Set(["docs/rule-syntax/lib/core.rerail"]) };

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

  it("keep a path given to node:path's resolve, and drop a module given to require's or import.meta's", () => {
    const text = ['resolve("scripts/x.mjs");', 'path.resolve("scripts/y.mjs");', 'require.resolve("y");', 'import.meta.resolve("z");'].join("\n");
    expect(literals(text)).toEqual(["scripts/x.mjs", "scripts/y.mjs"]);
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
    expect(seen(["native/src/lib.rs", "src/cli/cli.ts"], SOURCES).sort()).toEqual(["dist/cli/cli.js", "native/rerail_engine.node", "native/src/lib.rs", "src/cli/cli.ts"]);
    expect(seen(["Cargo.lock"], SOURCES)).toContain("native/rerail_engine.node");
  });

  it("is the addon for a crate the engine reaches and a file it embeds outside native/, not for the rest", () => {
    expect(seen(["tools/bitlex/src/lib.rs"], SOURCES)).toContain("native/rerail_engine.node");
    expect(seen(["docs/rule-syntax/lib/core.rerail"], SOURCES)).toContain("native/rerail_engine.node");
    expect(seen(["src/engine/native.ts"], SOURCES)).not.toContain("native/rerail_engine.node");
    expect(seen(["docs/rule-syntax/README.md"], SOURCES)).not.toContain("native/rerail_engine.node");
  });

  it("takes the engine's inputs from engine-inputs.mjs", () => {
    const sources = engineSources(ROOT);
    expect(isEngineInput("tools/bitlex/Cargo.toml", sources)).toBe(true);
    expect(isEngineInput("docs/rule-syntax/lib/core.rerail", sources)).toBe(true);
    expect(isEngineInput("src/cli/index.ts", sources)).toBe(false);
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
    expect(affected(tests, graph, ["src/lib.ts"], SOURCES)).toEqual(["test/a.test.ts"]);
    expect(affected(tests, graph, ["native/src/x.rs"], SOURCES)).toEqual(["test/b.test.ts"]);
    expect(affected(tests, graph, ["accel/src/y.rs"], SOURCES)).toEqual(["test/b.test.ts"]);
    expect(affected(tests, graph, ["tools/bitlex/src/z.rs"], SOURCES)).toEqual(["test/b.test.ts"]);
    expect(affected(tests, graph, ["scripts/helper.mjs"], SOURCES)).toEqual(["test/c.test.ts"]);
    expect(affected(tests, graph, ["docs/readme.md"], SOURCES)).toEqual([]);
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
    expect(outputs({ unit: "all", tests: [], all: 449, windowsExtra: true })["unit-shards"]).toBe("[1,2,3]");
  });

  it("run Linux's shards and no Windows shard for prose alone", () => {
    expect(outputs({ unit: "some", tests: ["test/a.test.ts"], all: 449, windowsExtra: true, prose: true })).toEqual({ unit: "some", "unit-tests": "test/a.test.ts", "unit-shards": "[1]", "windows-shards": "[]" });
    expect(outputs({ unit: "none", tests: [], all: 449, windowsExtra: true, prose: true })["windows-shards"]).toBe("[]");
  });

  // On this repository: vitest's list and every test file's import closure, which take seconds.
  it("give a change of prose alone no Windows shard, and one that also changes code its Windows shards", { timeout: 120_000 }, async () => {
    const prose = await select(ROOT, [".changeset/x.md", "docs/release.md"]);
    expect(prose.prose).toBe(true);
    expect(outputs(prose)["windows-shards"]).toBe("[]");
    const code = await select(ROOT, [".changeset/x.md", "docs/release.md", "src/cli/cli.ts"]);
    expect(code.prose).toBe(false);
    expect(JSON.parse(outputs(code)["windows-shards"]).length).toBeGreaterThan(0);
  });
});
