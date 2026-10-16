import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { join, posix } from "node:path";
import ts from "typescript";
import { describe, test } from "vitest";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { GATE_CODE } from "../../../conformance/port-readiness/cache.js";

const tracked = (pathspec: readonly string[]): string[] =>
  execFileSync("git", ["ls-files", "-z", "--", ...pathspec], { cwd: PACKAGE_ROOT, encoding: "utf8", maxBuffer: 1 << 28 }).split("\0").filter((path) => path !== "");

/**
 * Every file `entry` reaches through relative imports, by repository path: `x.js` is the `x.ts` beside its importer, as
 * tsx loads it. A relative specifier that resolves to no file fails, so a resolution this misreads cannot quietly
 * shrink the graph the key is checked against.
 */
function importGraph(entry: string): Set<string> {
  const reached = new Set<string>();
  const queue = [entry];
  while (queue.length > 0) {
    const file = queue.pop()!;
    if (reached.has(file)) continue;
    reached.add(file);
    for (const { fileName } of ts.preProcessFile(readFileSync(join(PACKAGE_ROOT, file), "utf8"), true, true).importedFiles) {
      if (!fileName.startsWith("./") && !fileName.startsWith("../")) continue;
      const target = posix.join(posix.dirname(file), fileName.endsWith(".js") ? `${fileName.slice(0, -3)}.ts` : fileName);
      const absolute = join(PACKAGE_ROOT, target);
      assert.ok(existsSync(absolute) && statSync(absolute).isFile(), `${file} imports ${fileName}, which resolves to no file`);
      queue.push(target);
    }
  }
  return reached;
}

describe("the gate code a rule's key covers", () => {
  const graph = importGraph("conformance/port-readiness/run.ts");
  const keyed = new Set(tracked(GATE_CODE));
  const excluded = tracked(["conformance/port-readiness"]).filter((path) => !keyed.has(path));

  test("run.ts's import graph is read whole", () => {
    for (const path of ["conformance/port-readiness/checks/matches-real-code.ts", "conformance/port-readiness/corpus.ts", "conformance/port-readiness/cache.ts"]) {
      assert.ok(graph.has(path), `run.ts's import graph misses ${path}`);
    }
  });

  // An excluded file run.ts reaches could change a verdict while every cached result of it stays standing.
  test("run.ts reaches no file the key leaves out", () => {
    assert.ok(excluded.length > 0, "GATE_CODE excludes nothing");
    for (const path of excluded) assert.ok(!graph.has(path), `${path} is out of the key, but run.ts imports it`);
  });

  // The lists are JSON, which no import reaches: a gate that read one by path would be judging with an unkeyed file.
  test("no gate code run.ts reaches names the not-gated or pending list", () => {
    for (const path of [...graph].filter((path) => path.startsWith("conformance/port-readiness/"))) {
      const lines = readFileSync(join(PACKAGE_ROOT, path), "utf8").split("\n").filter((line) => !line.includes(":(exclude)"));
      for (const name of ["ungated-rules.json", "pending.json"]) assert.ok(!lines.some((line) => line.includes(name)), `${path} names ${name}`);
    }
  });

  test("the run's options and the files run.ts reads by path stay in the key", () => {
    for (const path of ["conformance/port-readiness/cli.ts", "conformance/port-readiness/corpus-tasks.json"]) assert.ok(keyed.has(path), `${path} is out of the key`);
  });

  // A misspelt exclusion excludes nothing, and the file it meant stays in every key without anyone noticing.
  test("every exclusion names a tracked file", () => {
    for (const entry of GATE_CODE.filter((entry) => entry.startsWith(":(exclude)"))) {
      const path = entry.slice(":(exclude)".length);
      assert.deepEqual(tracked([path]), [path], `${entry} names no tracked file`);
    }
  });
});
