import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import type { CorpusFile } from "../../../conformance/port-readiness/corpus.js";
import type { CompiledPolicy, Engine } from "../../../conformance/port-readiness/engine.js";
import { GOLDENS_FILE, parseGoldens, readGoldens, serializeGoldens, SYSTEMATIC_FILES, type Golden, type Goldens } from "../../../conformance/port-readiness/goldens.js";
import { goldensGate, realCodeGapCheck } from "../../../conformance/port-readiness/checks/matches-upstream-tests.js";
import { goldenListLines } from "../../../conformance/port-readiness/run.js";
import type { PackAdapter, Span } from "../../../conformance/port-readiness/types.js";

// Sources are single lines of ASCII, so a byte offset is the column.
const span = (start: number, end: number): Span => ({ startLine: 0, startCol: start, endLine: 0, endCol: end });

const engine = { rerail: { dialectForPath: (path: string) => (path.endsWith(".ts") ? "typescript" : undefined) } } as unknown as Engine;
const adapter = { toSpan: (_name: string, _source: string, startByte: number, endByte: number) => span(startByte, endByte) } as unknown as PackAdapter;

/** `failures` maps a path to the engine failure code it reports there. */
function compiled(results: Record<string, [number, number][]>, failures: Record<string, string> = {}): CompiledPolicy {
  return {
    canonicalJson: "{}",
    digest: "sha256:0",
    inScope: (path) => !path.includes("vendor"),
    evaluateFile(path) {
      return {
        status: "completed",
        failures: failures[path] === undefined ? [] : [{ code: failures[path]!, message: `engine ${failures[path]}` }],
        violations: (results[path] ?? []).map(([startByte, endByte]) => ({ clauseId: "c", code: "C", path, span: { startByte, endByte } })),
      };
    },
  };
}

function corpusFile(task: string, path: string): CorpusFile {
  return { task, repository: "o/r", path, source: "debugger; debugger;", sha256: "0".repeat(64) };
}

const SHA = "0".repeat(64);
const golden = (task: string, path: string, sha256 = SHA): Golden => ({ task, path, sha256 });

test("run integrity names a stale entry, a rule with goldens and a gap, and a candidate that differs on goldens throughout", () => {
  const rules = [
    { id: "shipped-rule", location: "shipped" as const, differing: 0 },
    { id: "candidate-rule", location: "candidate" as const, differing: 2 },
    { id: "systematic-rule", location: "candidate" as const, differing: SYSTEMATIC_FILES + 1 },
    { id: "gapped-rule", location: "candidate" as const, differing: 0 },
  ];
  const goldens: Goldens = new Map([
    ["shipped-rule", [golden("t1", "a.ts")]],
    ["candidate-rule", [golden("t1", "a.ts"), golden("t1", "b.ts")]],
    ["systematic-rule", [golden("t1", "a.ts")]],
    ["gapped-rule", [golden("t1", "a.ts")]],
    ["gone-rule", [golden("t1", "a.ts")]],
  ]);
  assert.deepEqual(goldenListLines(goldens, [{ rule: "gapped-rule", differing_files: 900 }, { rule: "gone-gap", differing_files: 3 }], rules), [
    `gapped-rule has 1 golden(s) and a real_code_gaps entry: keep one, the goldens unless it differs on more than ${SYSTEMATIC_FILES} of them`,
    "goldens.tsv lists 1 golden(s) of gone-rule, which is no rule of this tool-pack",
    "gates.yaml lists a real-code gap for gone-gap, which is no rule of this tool-pack",
    `the candidate systematic-rule differs on ${SYSTEMATIC_FILES + 1} of its goldens, more than ${SYSTEMATIC_FILES}: list it under real_code_gaps in their place`,
  ]);
  // A candidate held back by a few goldens keeps them: that is how the gates see when it is fixed.
  assert.deepEqual(goldenListLines(new Map([["candidate-rule", [golden("t1", "a.ts")]]]), [], rules.slice(0, 2)), []);
});

test("a rule with a real-code gap fails no-real-code-gap, naming how many files NRV found it differing on", () => {
  assert.deepEqual(realCodeGapCheck(undefined), { id: "no-real-code-gap", ok: true, detail: "gates.yaml lists no real-code gap for the rule" });
  const failing = realCodeGapCheck({ rule: "typescript-eslint-consistent-type-imports", differing_files: 6898 });
  assert.equal(failing.ok, false);
  assert.match(failing.detail, /differs from upstream on 6898 of NRV's goldens for it/u);
});

test("the goldens file is one golden per line, grouped by rule and then task, every level sorted", () => {
  const a: Goldens = new Map([
    ["z:rule", [golden("t2", "b.ts", "1".repeat(64)), golden("t1", "a.ts"), golden("t1", "0.ts")]],
    ["a:rule", [golden("rerail:vscode", "src/x.ts")]],
    ["m:rule", []],
  ]);
  const b: Goldens = new Map([
    ["a:rule", [golden("rerail:vscode", "src/x.ts")]],
    ["z:rule", [golden("t1", "0.ts"), golden("t2", "b.ts", "1".repeat(64)), golden("t1", "a.ts")]],
  ]);
  const text = serializeGoldens(a);
  assert.equal(text, serializeGoldens(b));
  assert.deepEqual(text.split("\n").slice(1), [
    "a:rule",
    "\trerail:vscode",
    `\t\tsrc/x.ts\t${SHA}`,
    "z:rule",
    "\tt1",
    `\t\t0.ts\t${SHA}`,
    `\t\ta.ts\t${SHA}`,
    "\tt2",
    `\t\tb.ts\t${"1".repeat(64)}`,
    "",
  ]);
  assert.deepEqual([...parseGoldens(text, "goldens.tsv")], [...b].map(([id, entries]) => [id, [...entries].sort((x, y) => (x.task + x.path < y.task + y.path ? -1 : 1))]));
});

test("a goldens file not in its one form is refused", () => {
  const text = serializeGoldens(new Map([["r", [golden("t1", "a.ts"), golden("t1", "b.ts")]]]));
  const swapped = text.replace(`\t\ta.ts\t${SHA}\n\t\tb.ts`, `\t\tb.ts\t${SHA}\n\t\ta.ts`);
  assert.throws(() => parseGoldens(swapped, "goldens.tsv"), /not in its one form/u);
  assert.throws(() => parseGoldens(text.replace(SHA, "x"), "goldens.tsv"), /not a golden/u);
  assert.throws(() => serializeGoldens(new Map([["r", [golden("t1", "a\tb.ts")]]])), /cannot be written/u);
});

test("readGoldens reads a tool-pack's goldens.tsv, and a tool-pack without one has none", async () => {
  const root = await mkdtemp(join(tmpdir(), "validation-goldens-"));
  try {
    assert.equal((await readGoldens(root)).size, 0);
    const goldens: Goldens = new Map([["eslint:no-debugger", [golden("t1", "src/a.ts")]]]);
    await writeFile(join(root, GOLDENS_FILE), serializeGoldens(goldens));
    assert.deepEqual([...await readGoldens(root)], [...goldens]);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("identical-on-goldens requires exactly upstream's findings on every golden, in both directions", () => {
  const goldens = [
    { golden: golden("t1", "src/agree.ts"), file: corpusFile("t1", "src/agree.ts"), expected: [span(0, 9)] },
    { golden: golden("t1", "src/missing.ts"), file: corpusFile("t1", "src/missing.ts"), expected: [span(0, 9)] },
    { golden: golden("t1", "src/extra.ts"), file: corpusFile("t1", "src/extra.ts"), expected: [] },
  ];
  const rule = compiled({ "src/agree.ts": [[0, 9]], "src/extra.ts": [[10, 19]] });
  const result = goldensGate(engine, adapter, "no-debugger", rule, goldens);
  assert.equal(result.check.id, "identical-on-goldens");
  assert.equal(result.check.ok, false);
  assert.deepEqual(result.results.map((entry) => [entry.path, entry.status]), [["src/agree.ts", "agree"], ["src/missing.ts", "mismatch"], ["src/extra.ts", "mismatch"]]);
  assert.deepEqual(result.results[1]!.missing, [span(0, 9)]);
  assert.deepEqual(result.results[2]!.extra, [span(10, 19)]);
  assert.match(result.check.detail, /^2 of 3 golden\(s\) differ, e\.g\. t1\/src\/missing\.ts: missing 1:1-1:10$/u);
  const fixed = goldensGate(engine, adapter, "no-debugger", compiled({ "src/agree.ts": [[0, 9]], "src/missing.ts": [[0, 9]] }), goldens);
  assert.equal(fixed.check.ok, true);
  assert.equal(fixed.check.detail, "identical on all 3 golden(s)");
  assert.equal(goldensGate(engine, adapter, "no-debugger", rule, []).check.ok, true);
});

test("a golden fails outside the document's scope with upstream findings, or when it can no longer be checked", () => {
  const result = goldensGate(engine, adapter, "no-debugger", compiled({}), [
    { golden: golden("t1", "src/vendor.ts"), file: corpusFile("t1", "src/vendor.ts"), expected: [span(0, 9)] },
    { golden: golden("t1", "src/vendor/quiet.ts"), file: corpusFile("t1", "src/vendor/quiet.ts"), expected: [] },
    { golden: golden("t1", "src/gone.ts"), file: undefined, expected: null },
    { golden: golden("t1", "src/rejected.ts"), file: corpusFile("t1", "src/rejected.ts"), expected: null },
  ]);
  assert.deepEqual(result.results.map((entry) => entry.status), ["mismatch", "agree", "mismatch", "mismatch"]);
  assert.match(result.results[2]!.reason, /^the pinned corpus has no t1\/src\/gone\.ts with sha256 0+$/u);
  assert.match(result.results[3]!.reason, /upstream's own parser rejects it/u);
  // A golden corpus.json does not answer says so, so the fix (record the tool-pack again) is plain.
  const unrecorded = goldensGate(engine, adapter, "no-debugger", compiled({}), [
    { golden: golden("t1", "src/new.ts"), file: corpusFile("t1", "src/new.ts"), expected: null, excluded: "corpus.json has no answer for it with this sha256, so the tool-pack needs recording again" },
  ]);
  assert.equal(unrecorded.results[0]!.reason, "it has no expected answer: corpus.json has no answer for it with this sha256, so the tool-pack needs recording again");
});

test("a golden the engine could not fully read is an engine gap, which does not fail the rule", () => {
  const goldens = [
    { golden: golden("t1", "src/partial.ts"), file: corpusFile("t1", "src/partial.ts"), expected: [span(0, 9)] },
    { golden: golden("t1", "src/incomplete.ts"), file: corpusFile("t1", "src/incomplete.ts"), expected: [] },
    { golden: golden("t1", "src/budget.ts"), file: corpusFile("t1", "src/budget.ts"), expected: [] },
    { golden: golden("t1", "src/agree.ts"), file: corpusFile("t1", "src/agree.ts"), expected: [span(0, 9)] },
  ];
  const failures = { "src/partial.ts": "source-partial", "src/incomplete.ts": "source-incomplete", "src/budget.ts": "budget-exceeded" };
  const result = goldensGate(engine, adapter, "no-debugger", compiled({ "src/agree.ts": [[0, 9]] }, failures), goldens);
  assert.deepEqual(result.results.map((entry) => entry.status), ["engine-gap", "engine-gap", "engine-gap", "agree"]);
  assert.equal(result.results[0]!.reason, "engine gap: missing 1:1-1:10");
  assert.equal(result.results[2]!.reason, "engine gap: engine budget-exceeded");
  assert.equal(result.check.ok, true);
  assert.equal(result.check.detail, "identical on 1 of 4 golden(s); the engine could not fully read the other 3 (engine gaps)");
  // A parse failure is listed for the independent parsers; a budget overrun is not a parse failure.
  assert.deepEqual(result.unparsed.map((entry) => entry.file), ["t1/src/partial.ts", "t1/src/incomplete.ts"]);
});

test("a golden that differs on a file the engine read fully still fails the rule, beside engine gaps", () => {
  const result = goldensGate(engine, adapter, "no-debugger", compiled({ "src/extra.ts": [[10, 19]] }, { "src/incomplete.ts": "source-incomplete" }), [
    { golden: golden("t1", "src/incomplete.ts"), file: corpusFile("t1", "src/incomplete.ts"), expected: [] },
    { golden: golden("t1", "src/extra.ts"), file: corpusFile("t1", "src/extra.ts"), expected: [] },
  ]);
  assert.deepEqual(result.results.map((entry) => entry.status), ["engine-gap", "mismatch"]);
  assert.equal(result.check.ok, false);
  assert.equal(result.check.detail, "1 of 2 golden(s) differ, e.g. t1/src/extra.ts: extra 1:11-1:20; 1 engine gap(s)");
});
