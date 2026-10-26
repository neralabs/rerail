/**
 * The conformance runner's real-code samples (`runSamples`, `scoreSamples` and `reportFailures`,
 * conformance/languages/), and the passes-conformance gate's samples checks: samples are judged by intent, line only.
 * A labelled violation must be caught on its line, a clean sample must get no finding, a judged sample must be read;
 * an extra report on a labelled sample is only reviewed. Trees are hand-made and reached through `samplesDirectory`.
 */
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { loadPack, type Divergence } from "../../conformance/languages/pack.js";
import type { RuleRow, T4Report } from "../../conformance/languages/report.js";
import { reportFailures, runPack, type RunOptions } from "../../conformance/languages/runner.js";
import { conformanceOf, passesConformanceChecks } from "../../conformance/port-readiness/checks/passes-conformance.js";
import { fixturesPath } from "../support/oracles.js";
import { writeSamplesTree, type HandMadeSample } from "../support/samples.js";

const DATE = "2026-01-01";
const UNREADABLE = `assert ready\n${"This is not Python: it is prose, ) and ( brackets ] everywhere.\n".repeat(6)}`;

const editJson = (path: string, edit: (value: Record<string, unknown>) => unknown): void =>
  writeFileSync(path, JSON.stringify(edit(JSON.parse(readFileSync(path, "utf8")) as Record<string, unknown>), null, 2));

const samplesRows = (report: T4Report) =>
  report.rules
    .filter(row => row.source === "samples")
    .map(({ rule, candidate, tp, fp, fn, reviewed, cleanSamples, silentCleanSamples }) => ({ rule, candidate, tp, fp, fn, reviewed, cleanSamples, silentCleanSamples }));

const samplesRow = (report: T4Report): RuleRow => report.rules.find(row => row.source === "samples")!;

/** The passes-conformance checks assert-used fails, from the report the gate would read. */
const failingChecks = (report: T4Report): string[] =>
  passesConformanceChecks(conformanceOf(loadPack(directory), report), "assert-used")
    .filter(check => !check.ok)
    .map(check => check.id);

let directory: string;
let tree: string;

beforeEach(() => {
  // The `valid` pack ports examplelint's EX101 (every assert) as rules/assert-used.json, matched at line+column on the
  // fixtures; samples are judged at the line alone.
  directory = mkdtempSync(join(tmpdir(), "rerail-samples-"));
  cpSync(fixturesPath("packs", "valid"), directory, { recursive: true });
  tree = join(directory, "samples-tree");
});

afterEach(() => rmSync(directory, { recursive: true, force: true }));

/** Writes `samples` as examplelint's tree, then scores the pack on its samples (and the fixtures, with `fixtures`). */
async function score(samples: readonly HandMadeSample[], options: Partial<RunOptions> = {}): Promise<{ paths: string[]; report: T4Report }> {
  const paths = writeSamplesTree(tree, samples);
  const report = await runPack(loadPack(directory), {
    fixtures: false,
    samples: true,
    samplesDirectory: tool => (tool === "examplelint" ? tree : undefined),
    date: DATE,
    ...options,
  });
  return { paths, report };
}

const writeDivergences = (divergences: Divergence[]): void => {
  const existing = JSON.parse(readFileSync(join(directory, "divergences.json"), "utf8")) as Divergence[];
  writeFileSync(join(directory, "divergences.json"), JSON.stringify([...existing, ...divergences], null, 2));
};

/** A second rule, porting `PASS` from `tool`: rules/pass-statement.json, which flags every pass statement. */
const addPassStatement = (tool: string): void =>
  editJson(join(directory, "conformance.json"), pack => ({
    ...pack,
    tools: tool === "examplelint" ? pack.tools : [...(pack.tools as unknown[]), { name: tool, version: "1.0.0" }],
    rules: [...(pack.rules as unknown[]), { id: "pass-statement", upstream: { tool, rule: "PASS" }, policy: "rules/pass-statement.json", capabilities: ["syntax"], match: "line" }],
  }));

describe("a shipped rule on its samples", () => {
  it("catches a labelled violation the rule reports on that line", async () => {
    const { report } = await score([{ name: "caught", path: "app.py", text: "ready = True\nassert ready, 'not ready'\n", expected: { "assert-used": [2] } }]);
    expect(samplesRows(report)).toEqual([{ rule: "assert-used", candidate: false, tp: 1, fp: 0, fn: 0, reviewed: 0, cleanSamples: 0, silentCleanSamples: 0 }]);
    expect(report.sources).toEqual([expect.objectContaining({ source: "samples", units: 1, files: 1 })]);
    expect(reportFailures(report)).toEqual([]);
  });

  it("fails on a missed label", async () => {
    const { paths, report } = await score([{ name: "missed", path: "app.py", text: "ready = True\n", expected: { "assert-used": [1] } }], { fixtures: true });
    expect(samplesRows(report)).toEqual([expect.objectContaining({ tp: 0, fn: 1 })]);
    expect(reportFailures(report)).toEqual([`assert-used (samples): 1 missed label(s), first {"path":"${paths[0]}","line":1}`]);
    expect(failingChecks(report)).toEqual(["samples-catch-labelled"]);
  });

  it("fails on a finding on a clean sample", async () => {
    const { paths, report } = await score([{ name: "clean", path: "app.py", text: "assert ready\n", expected: { "assert-used": [] } }], { fixtures: true });
    expect(samplesRows(report)).toEqual([expect.objectContaining({ fp: 1, cleanSamples: 1, silentCleanSamples: 0 })]);
    expect(reportFailures(report)).toEqual([`assert-used (samples): 1 finding(s) on clean samples, first {"path":"${paths[0]}","line":1}`]);
    expect(failingChecks(report)).toEqual(["samples-silent-on-clean"]);
  });

  it("reports an extra finding on a labelled sample for review, without failing", async () => {
    const { paths, report } = await score([{ name: "extra", path: "app.py", text: "assert a\nx = 1\nassert b\n", expected: { "assert-used": [1] } }], { fixtures: true });
    expect(samplesRows(report)).toEqual([expect.objectContaining({ tp: 1, fp: 0, fn: 0, reviewed: 1 })]);
    expect(samplesRow(report).reviewedFindings).toEqual([{ path: paths[0], line: 3 }]);
    expect(reportFailures(report)).toEqual([]);
    expect(failingChecks(report)).toEqual([]);
  });

  it("fails on a judged sample the engine cannot read", async () => {
    const { report } = await score([{ name: "unread", path: "app.py", text: UNREADABLE, expected: { "assert-used": [] } }], { fixtures: true });
    expect(samplesRow(report)).toMatchObject({ fp: 0, engineFailures: 0, cleanSamples: 1, silentCleanSamples: 0, syntaxErrorFiles: { count: 1 } });
    expect(reportFailures(report)).toEqual([expect.stringMatching(/^assert-used \(samples\): 1 judged sample\(s\) the engine could not read, first /u)]);
    expect(failingChecks(report)).toEqual(["engine-reads-samples"]);
  });

  it("does not judge a sample for a rule it has no key for", async () => {
    addPassStatement("examplelint");
    const text = "def f():\n    pass\nassert ready\n";
    const { report } = await score([
      { name: "asserts-only", path: "app.py", text, expected: { "assert-used": [3] } },
      { name: "passes-only", path: "lib.py", text: "def g():\n    pass\n", expected: { "pass-statement": [2] } },
    ]);
    expect(samplesRows(report).map(({ rule, tp, fp, fn }) => ({ rule, tp, fp, fn }))).toEqual([
      { rule: "assert-used", tp: 1, fp: 0, fn: 0 },
      // app.py's pass is no finding of pass-statement: app.py has no key for it.
      { rule: "pass-statement", tp: 1, fp: 0, fn: 0 },
    ]);
    expect(reportFailures(report)).toEqual([]);
  });

  it("is an error for a label naming no rule of the tool", async () => {
    const { paths, report } = await score([{ name: "stray", path: "app.py", text: "x = 1\n", expected: { "no-such-rule": [] } }]);
    expect(report.errors).toEqual([`samples examplelint: ${paths[0]} is labelled for no-such-rule, no examplelint rule of the valid pack`]);
  });
});

describe("a candidate on its samples", () => {
  it("is reported, failing nothing", async () => {
    mkdirSync(join(directory, "candidates"), { recursive: true });
    cpSync(join(directory, "rules", "assert-used.json"), join(directory, "candidates", "assert-used.json"));
    editJson(join(directory, "conformance.json"), pack => ({
      ...pack,
      rules: (pack.rules as Record<string, unknown>[]).map(rule => ({ ...rule, policy: "candidates/assert-used.json" })),
    }));
    const { report } = await score([
      { name: "missed", path: "app.py", text: "ready = True\n", expected: { "assert-used": [1] } },
      { name: "clean", path: "lib.py", text: "assert ready\n", expected: { "assert-used": [] } },
    ]);
    expect(samplesRows(report)).toEqual([expect.objectContaining({ candidate: true, fp: 1, fn: 1 })]);
    expect(reportFailures(report)).toEqual([]);
    expect(report.pass).toBe(true);
  });
});

describe("samples divergences", () => {
  const evidence = "checked by hand against the source: the label is wrong here";

  it("explain a finding on a clean sample and a missed label, by evaluation path and line", async () => {
    const samples: HandMadeSample[] = [
      { name: "missed", path: "app.py", text: "ready = True\n", expected: { "assert-used": [1] } },
      { name: "clean", path: "lib.py", text: "assert ready\n", expected: { "assert-used": [] } },
    ];
    const paths = writeSamplesTree(tree, samples);
    writeDivergences([
      { rule: "assert-used", source: "samples", path: paths[0]!, line: 1, kind: "rerail-unknown", evidence },
      { rule: "assert-used", source: "samples", path: paths[1]!, line: 1, kind: "upstream-wrong", evidence },
    ]);
    const { report } = await score(samples);
    expect(samplesRows(report)).toEqual([expect.objectContaining({ fp: 0, fn: 0 })]);
    expect(report.rules[0]!.divergences.map(({ side, path, line }) => ({ side, path, line }))).toEqual(
      expect.arrayContaining([{ side: "fn", path: paths[0], line: 1 }, { side: "fp", path: paths[1], line: 1 }]),
    );
    expect(reportFailures(report)).toEqual([]);
  });

  it("fail once they explain nothing; one for every source only once both sources scored the rule", async () => {
    const samples: HandMadeSample[] = [{ name: "caught", path: "app.py", text: "assert ready\n", expected: { "assert-used": [1] } }];
    const paths = writeSamplesTree(tree, samples);
    writeDivergences([{ rule: "assert-used", source: "samples", path: paths[0]!, line: 5, kind: "upstream-wrong", evidence }]);
    expect(reportFailures((await score(samples)).report)).toEqual([`assert-used: stale divergence ${paths[0]}:5 (upstream-wrong) explains nothing`]);

    writeFileSync(join(directory, "divergences.json"), readFileSync(fixturesPath("packs", "valid", "divergences.json")));
    writeDivergences([{ rule: "assert-used", path: "nowhere.py", line: 1, kind: "upstream-wrong", evidence }]);
    expect((await score(samples)).report.staleDivergences).toEqual([]);
    expect((await score(samples, { fixtures: true })).report.staleDivergences).toEqual([{ rule: "assert-used", path: "nowhere.py", line: 1, kind: "upstream-wrong" }]);
  });
});

describe("stays-silent", () => {
  const staysSilent = (report: T4Report) =>
    passesConformanceChecks(conformanceOf(loadPack(directory), report), "assert-used").find(check => check.id === "stays-silent")!;
  /** The fixtures recording without clean.py, its one unit where examplelint reports nothing. */
  const withoutCleanFixture = (): void =>
    editJson(join(directory, "upstream", "examplelint", "fixtures.expected.json"), recording => ({
      ...recording,
      source: { ...(recording.source as Record<string, unknown>), files: ["asserts.py"] },
    }));
  const clean: HandMadeSample = { name: "clean", path: "app.py", text: "ready = True\n", expected: { "assert-used": [] } };

  it("is met by a fixture unit upstream reports nothing on, where the rule is silent", async () => {
    const report = await runPack(loadPack(directory), { fixtures: true, samples: false, date: DATE });
    const fixtures = report.rules.find(row => row.source === "fixtures") as RuleRow;
    expect({ clean: fixtures.cleanSamples, silent: fixtures.silentCleanSamples }).toEqual({ clean: 1, silent: 1 });
    expect(staysSilent(report)).toMatchObject({ ok: true, detail: "silent where upstream reports nothing on 1 fixture unit(s), and on 0 clean sample(s)" });
  });

  it("is met by a clean sample the rule is silent on", async () => {
    withoutCleanFixture();
    const { report } = await score([clean], { fixtures: true });
    expect(samplesRows(report)).toEqual([expect.objectContaining({ cleanSamples: 1, silentCleanSamples: 1 })]);
    expect(staysSilent(report)).toMatchObject({ ok: true, detail: "silent where upstream reports nothing on 0 fixture unit(s), and on 1 clean sample(s)" });
  });

  it("fails with neither", async () => {
    withoutCleanFixture();
    const { report } = await score([{ ...clean, text: UNREADABLE }], { fixtures: true });
    expect(staysSilent(report).ok).toBe(false);
  });
});

describe("a missing samples tree", () => {
  it("fails only the rules of its tool", async () => {
    addPassStatement("otherlint");
    const samples: HandMadeSample[] = [{ name: "caught", path: "app.py", text: "assert ready\n", expected: { "assert-used": [1] } }];
    writeSamplesTree(tree, samples);
    writeDivergences([{ rule: "pass-statement", source: "samples", path: "x/app.py", line: 1, kind: "upstream-wrong", evidence: "checked by hand: the label is wrong here" }]);
    const missing = join(directory, "not-restored");
    for (const otherlint of [() => missing, () => { throw new Error("s3://bucket/key is not in S3"); }]) {
      const report = await runPack(loadPack(directory), {
        fixtures: false,
        samples: true,
        samplesDirectory: tool => (tool === "examplelint" ? tree : otherlint()),
        date: DATE,
      });
      expect(report.errors).toEqual([expect.stringMatching(/^pass-statement \(samples\): (no samples tree at .*not-restored|s3:\/\/bucket\/key is not in S3)$/u)]);
      // The rule's samples divergence is not stale: its samples were never scored.
      expect(report.staleDivergences).toEqual([]);
      expect(samplesRows(report).map(({ rule, tp }) => ({ rule, tp }))).toEqual([{ rule: "assert-used", tp: 1 }]);
      const conformance = conformanceOf(loadPack(directory), report);
      expect(conformance.errors).toEqual([]);
      expect([...conformance.ruleErrors.keys()]).toEqual(["pass-statement"]);
    }
  });
});

describe("a sample's context files", () => {
  beforeEach(() => {
    // The `valid` pack, made an ECMA pack whose one port reports every file of a package that depends on next.
    writeFileSync(join(directory, "conformance.json"), JSON.stringify({
      pack: "valid",
      family: "ecma",
      capabilities: ["syntax"],
      corpora: [],
      tools: [{ name: "examplelint", version: "1.0.0" }],
      rules: [{ id: "next-file", upstream: { tool: "examplelint", rule: "EX200" }, policy: "rules/next-file.json", capabilities: ["syntax"], match: "line+column" }],
    }));
    writeFileSync(join(directory, "divergences.json"), "[]\n");
  });

  const writeRule = (file: Record<string, unknown>): void =>
    writeFileSync(join(directory, "rules", "next-file.json"), JSON.stringify({
      format: "rerail.policy/v1", id: "valid:next-file", revision: 1, mode: "steer",
      scope: { include: ["**/*"], exclude: [] },
      clauses: [{ id: "next-file", forbid: { ast: { language: "typescript", file, rule: { kind: "program" } } }, violation: { anchor: "witness.span", code: "EX200", message: "examplelint EX200" } }],
    }));

  it("give the judged file their manifest facts, and are never judged themselves", async () => {
    writeRule({ package: { dependsOn: ["next"] } });
    const { report } = await score([
      { name: "page", path: "app/page.js", text: "export {};\n", expected: { "next-file": [1] }, context: { "package.json": '{ "dependencies": { "next": "15" } }\n', "app/other.js": "export {};\n" } },
    ]);
    expect(report.errors).toEqual([]);
    expect(samplesRows(report)).toEqual([expect.objectContaining({ tp: 1, fp: 0, fn: 0, reviewed: 0 })]);
    expect(report.sources).toEqual([expect.objectContaining({ source: "samples", units: 1, files: 3 })]);
  });

  it("give the judged file the jsxImportSource of a project configuration's base", async () => {
    // React and next listed, so only the configuration's Preact runtime makes the file `other`.
    writeRule({ jsxRuntime: ["other"] });
    const { report } = await score([
      {
        name: "page",
        path: "app/page.js",
        text: "export {};\n",
        expected: { "next-file": [1] },
        context: {
          "package.json": '{ "dependencies": { "next": "15", "react": "19" } }\n',
          "tsconfig.json": '{ "extends": "./config/tsconfig-base.json" }\n',
          "config/tsconfig-base.json": '{ "compilerOptions": { "jsxImportSource": "preact", } }\n',
        },
      },
    ]);
    expect(report.errors).toEqual([]);
    expect(samplesRows(report)).toEqual([expect.objectContaining({ tp: 1, fn: 0 })]);
  });
});
