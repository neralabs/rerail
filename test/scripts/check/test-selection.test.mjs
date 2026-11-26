// scripts/check/ci/test-selection.mjs: which unit test files a change can affect, and the shards that run them.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";
import { engineSources, isEngineInput } from "../../../scripts/check/ci/engine-inputs.mjs";
import { affected, namedPaths, names, outputs, SHARDS, seen, select, shards, stringLiterals } from "../../../scripts/check/ci/test-selection.mjs";
import { fixturesPath } from "../../support/oracles.ts";
import { assignShards, platformTimes, runMs } from "../../support/shards/shard-split.ts";
import { UNIT_TEST_TIMES } from "../../support/shards/unit-test-times.generated.ts";

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
  // Ten files, 1,000 ms on each platform: one heavy file and nine light ones.
  const light = ["l1", "l2", "l3", "l4", "l5", "l6", "l7", "l8", "l9"].map((name) => `${name}.test.ts`);
  const files = { "h.test.ts": 910, ...Object.fromEntries(light.map((path) => [path, 10])) };
  const TIMES = { linux: { firstShardExtraMs: 0, files }, win32: { firstShardExtraMs: 0, files } };
  // Each unit job's platform in the times, and its output.
  const JOBS = [
    ["linux", "linux", "unit-shards"],
    ["windows", "win32", "windows-shards"],
  ];

  it("run as many shards as the larger of the selection's share of the files and of the job's time needs, at least one", () => {
    expect(shards("windows", ["h.test.ts"], 10, TIMES)).toEqual([1, 2, 3, 4, 5]);
    expect(shards("linux", ["h.test.ts"], 10, TIMES)).toEqual([1, 2, 3]);
    expect(shards("windows", light.slice(0, 5), 10, TIMES)).toEqual([1, 2, 3]);
    expect(shards("linux", light.slice(0, 5), 10, TIMES)).toEqual([1, 2]);
    expect(shards("windows", [], 10, TIMES)).toEqual([1]);
    expect(shards("windows", Object.keys(files), 10, TIMES)).toEqual([1, 2, 3, 4, 5]);
  });

  // The 186 of 552 test files CI selected for a change to the daemon's tests: a third of the files and three fifths of
  // Windows' time. Counted by the files, two Windows shards each held half of it, and one was cancelled at 15 minutes.
  it("hold no more time in a shard of a selection heavy by time than in a shard of every file", () => {
    const heavy = readFileSync(fixturesPath("host", "ci-shards", "heavy-selection.txt"), "utf8").split("\n").filter(Boolean);
    for (const [job, platform, output] of JOBS) {
      const times = platformTimes(UNIT_TEST_TIMES, platform);
      const count = JSON.parse(outputs({ unit: "some", tests: heavy, all: 552, windowsExtra: false })[output]).length;
      const wholeShard = runMs(Object.keys(times.files), times) / SHARDS[job];
      expect.soft(runMs(heavy, times) / count, `${job}: ${count} shards`).toBeLessThanOrEqual(wholeShard);
    }
  });

  // As the sequencer splits it, a shard of the selection holds no more than a shard of every file plus one file: the
  // split is even up to the last file it places.
  it("hold, in each shard of a selection the sequencer splits, at most a whole run's shard and one file", () => {
    for (const [job, platform, output] of JOBS) {
      const times = platformTimes(UNIT_TEST_TIMES, platform);
      const byPath = Object.keys(times.files).sort();
      const byTime = [...byPath].sort((a, b) => times.files[b] - times.files[a]);
      const tenth = Math.ceil(byPath.length / 10);
      const selections = [
        ...[1, 2, 5, 10, 20, 50, 100, 200, 300, 400].flatMap((k) => [byTime.slice(0, k), byTime.slice(-k)]),
        ...Array.from({ length: 10 }, (_, i) => byPath.slice(i * tenth, (i + 1) * tenth)),
      ];
      const wholeShard = runMs(byPath, times) / SHARDS[job];
      for (const selection of selections) {
        const count = JSON.parse(outputs({ unit: "some", tests: selection, all: byPath.length, windowsExtra: false })[output]).length;
        const loads = Array.from({ length: count }, (_, shard) => (shard === 0 ? times.firstShardExtraMs : 0));
        for (const [path, shard] of assignShards(selection, times, count)) loads[shard] += times.files[path];
        const heaviest = Math.max(...selection.map((path) => times.files[path]));
        expect(Math.max(...loads), `${job}: ${selection.length} files on ${count} shards`).toBeLessThanOrEqual(wholeShard + heaviest);
      }
    }
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
