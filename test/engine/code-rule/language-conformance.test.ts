/**
 * The language-pack conformance runner (docs/design/rerail-language-conformance.md): the sample pack pins its scoring
 * end to end, through the production workspace path. Every pack's own ports are scored by test/engine/catalog/packs.test.ts (check 4).
 */
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { loadPack, packSchema, type CapabilityExemption, type Divergence } from "../../../conformance/languages/pack.js";
import { reportFailures, runPack } from "../../../conformance/languages/runner.js";
import { precisionRecall, scoreSamples, scoreSource } from "../../../conformance/languages/score.js";
import { fixturesPath } from "../../support/oracles.js";
import { writeSamplesTree, type HandMadeSample } from "../../support/samples.js";

const SAMPLE = fixturesPath("packs", "sample-php");
const DATE = "2026-01-01";
const FIXTURES = { fixtures: true, samples: false, date: DATE } as const;
const SIZE = "<?php\n\nnamespace Mini;\n\nfinal class Size\n{\n    public function of(array $items): int\n    {\n        return \\sizeof($items);\n    }\n}\n";

/** Scores the sample pack on phpcs samples written to a temporary tree, and the fixtures with `fixtures`. */
async function withSamples(samples: readonly HandMadeSample[], fixtures: boolean) {
  const tree = mkdtempSync(join(tmpdir(), "language-conformance-"));
  try {
    writeSamplesTree(tree, samples);
    return await runPack(loadPack(SAMPLE), { fixtures, samples: true, samplesDirectory: tool => (tool === "phpcs" ? tree : undefined), date: DATE });
  } finally {
    rmSync(tree, { recursive: true, force: true });
  }
}

describe("language conformance runner", () => {
  it("scores the sample ports against PHP_CodeSniffer's recordings on fixtures, and on a labelled sample", async () => {
    const report = await withSamples([{ name: "mini", path: "src/Size.php", text: SIZE, expected: { "forbidden-functions": [9], "forbidden-functions-resolved": [9] } }], true);
    expect(report.rules.map(({ rule, source, tp, fp, fn, precision, recall }) => ({ rule, source, tp, fp, fn, precision, recall }))).toEqual([
      // The regex port runs around broken.php's syntax error and finds its sizeof call.
      { rule: "forbidden-functions", source: "fixtures", tp: 6, fp: 0, fn: 0, precision: 1, recall: 1 },
      // Explained misses lower recall: broken.php's call (rerail-unknown) and the namespaced delete (not-comparable).
      { rule: "forbidden-functions-resolved", source: "fixtures", tp: 4, fp: 0, fn: 0, precision: 1, recall: 4 / 6 },
      { rule: "forbidden-functions", source: "samples", tp: 1, fp: 0, fn: 0, precision: 1, recall: 1 },
      { rule: "forbidden-functions-resolved", source: "samples", tp: 1, fp: 0, fn: 0, precision: 1, recall: 1 },
    ]);
    expect(report.pass).toBe(true);
  });

  it("attributes a clause skipped for the file's syntax errors to T1, not to engine failures", async () => {
    const sample = loadPack(SAMPLE);
    const report = await runPack(sample, FIXTURES);
    const broken = { count: 1, files: [{ path: "broken.php", line: 6, column: 1 }] };
    expect(report.sources[0]?.syntaxErrorFiles).toEqual(broken);
    expect(report.rules.map(({ rule, engineFailures, partialFiles, syntaxErrorFiles }) => ({ rule, engineFailures, partialFiles, syntaxErrorFiles }))).toEqual([
      // Sound under errors: it ran around the error region (`source-partial`).
      { rule: "forbidden-functions", engineFailures: 0, partialFiles: 1, syntaxErrorFiles: { count: 0, files: [] } },
      // Name resolution is unsound under errors: it skipped the file (`source-incomplete`).
      { rule: "forbidden-functions-resolved", engineFailures: 0, partialFiles: 0, syntaxErrorFiles: broken },
    ]);
    // The skipped file's upstream finding is still a miss: without its divergence it fails the fixture tier.
    const undocumented = await runPack({ ...sample, divergences: sample.divergences.filter(divergence => divergence.path !== "broken.php") }, FIXTURES);
    expect(reportFailures(undocumented)).toEqual(['forbidden-functions-resolved (fixtures): 1 unexplained FN, first {"path":"broken.php","line":3,"column":6}']);
  });

  it("judges a sample the grammar did not read: the unread file fails nothing, the label it hides does", async () => {
    // Prose after the first statement: error regions cover 92% of the file, so no AST clause reads it, and the `sizeof`
    // call on line 2 is not reported. The unread file is T1's and reported; the label on line 2 it hides is a missed label.
    const garbage = `<?php\n$a = sizeof($b);\n${"This is not PHP: it is prose, ) and ( brackets ] everywhere.\n".repeat(6)}`;
    const report = await withSamples([{ name: "garbage", path: "src/Garbage.php", text: garbage, expected: { "forbidden-functions": [2], "forbidden-functions-resolved": [] } }], false);
    const [path] = report.rules.flatMap(row => row.syntaxErrorFiles.files.map(file => file.path));
    const unread = { count: 1, files: [{ path, line: 3, column: 6 }] };
    expect(report.rules.map(({ rule, tp, fp, fn, engineFailures, partialFiles, syntaxErrorFiles }) => ({ rule, tp, fp, fn, engineFailures, partialFiles, syntaxErrorFiles }))).toEqual([
      { rule: "forbidden-functions", tp: 0, fp: 0, fn: 1, engineFailures: 0, partialFiles: 0, syntaxErrorFiles: unread },
      { rule: "forbidden-functions-resolved", tp: 0, fp: 0, fn: 0, engineFailures: 0, partialFiles: 0, syntaxErrorFiles: unread },
    ]);
    expect(path).toMatch(/\/src\/Garbage\.php$/u);
    expect(reportFailures(report)).toEqual([`forbidden-functions (samples): 1 missed label(s), first ${JSON.stringify({ path, line: 2 })}`]);
  });

  it("fails on a divergence that no longer explains anything", async () => {
    const sample = loadPack(SAMPLE);
    const stale: Divergence = { rule: "forbidden-functions", source: "fixtures", path: "forbidden.php", line: 4, kind: "upstream-wrong", evidence: "documented when Rerail missed this call" };
    const loaded = { ...sample, divergences: [...sample.divergences, stale] };
    const fixturesOnly = await runPack(loaded, FIXTURES);
    expect(fixturesOnly.pass).toBe(false);
    expect(reportFailures(fixturesOnly)).toEqual(["forbidden-functions: stale divergence forbidden.php:4 (upstream-wrong) explains nothing"]);
    // A divergence for every source is judged only once every source has been scored.
    const { source: _fixtures, ...everySource } = stale;
    expect((await runPack({ ...sample, divergences: [...sample.divergences, everySource] }, FIXTURES)).staleDivergences).toEqual([]);
  });

  describe("capability exemptions", () => {
    const raw = JSON.parse(readFileSync(join(SAMPLE, "conformance.json"), "utf8")) as Record<string, unknown>;
    const exemption = (capability: string, coveredBy = ["T2: fixtures/php/calls/", "T3: benchmark_php_calls_corpus"]): CapabilityExemption => ({
      capability,
      reason: "no pinned linter reads callees transitively",
      evidence: "PHPStan 2.2.16, Psalm 6.19.1 and PHPCS 4.0.4 rules read only the direct callee's declaration",
      coveredBy,
    });
    const withExemption = (capabilities: string[], exemptions: CapabilityExemption[]) =>
      ({ ...loadPack(SAMPLE), pack: packSchema.parse({ ...raw, capabilities, exemptCapabilities: exemptions }) });

    it("covers a declared capability no rule exercises", async () => {
      const report = await runPack(withExemption(["syntax", "scope", "calls"], [exemption("calls")]), FIXTURES);
      expect(report.coverage.uncovered).toEqual([]);
      expect(report.coverage.exempt.map(entry => entry.capability)).toEqual(["calls"]);
      expect(report.pass).toBe(true);
    });

    it("is stale once a rule exercises the capability", async () => {
      const report = await runPack(withExemption(["syntax", "scope"], [exemption("syntax")]), FIXTURES);
      expect(reportFailures(report)).toEqual(["stale exemption: capability syntax is covered by rule(s) forbidden-functions"]);
    });

    it("needs a declared capability and both T2 and T3 coverage", () => {
      expect(() => withExemption(["syntax", "scope"], [exemption("calls")])).toThrow(/does not declare/u);
      expect(() => withExemption(["syntax", "scope", "calls"], [exemption("calls", ["T2: fixtures/php/calls/"])])).toThrow(/one T2 and one T3/u);
      expect(() => withExemption(["syntax", "scope", "calls"], [{ ...exemption("calls"), evidence: " " }])).toThrow();
    });
  });
});

describe("scoreSource", () => {
  const divergence = (kind: Divergence["kind"], line: number): Divergence => ({ rule: "r", path: "a.php", line, kind, evidence: "checked against the PHP runtime" });

  it("matches as multisets at the rule's granularity", () => {
    const expected = [
      { path: "a.php", line: 1, column: 1 },
      { path: "a.php", line: 1, column: 1 },
      { path: "a.php", line: 2, column: 3 },
    ];
    const actual = [
      { path: "a.php", line: 1, column: 1 },
      { path: "a.php", line: 2, column: 4 },
    ];
    const byColumn = scoreSource(expected, actual, "line+column", []);
    expect([byColumn.tp, byColumn.falsePositives, byColumn.falseNegatives]).toEqual([
      1,
      [{ path: "a.php", line: 2, column: 4 }],
      [
        { path: "a.php", line: 1, column: 1 },
        { path: "a.php", line: 2, column: 3 },
      ],
    ]);
    const strip = (points: typeof expected) => points.map(({ path, line }) => ({ path, line }));
    const byLine = scoreSource(strip(expected), strip(actual), "line", []);
    expect([byLine.tp, byLine.falsePositives.length, byLine.falseNegatives]).toEqual([2, 0, [{ path: "a.php", line: 1 }]]);
  });

  it("explains disagreements by divergence; rerail-unknown never excuses a claimed finding", () => {
    const score = scoreSource(
      [{ path: "a.php", line: 5 }],
      [
        { path: "a.php", line: 1 },
        { path: "a.php", line: 2 },
      ],
      "line",
      [divergence("upstream-wrong", 1), divergence("rerail-unknown", 2), divergence("rerail-unknown", 5)],
    );
    expect(score.falsePositives).toEqual([{ path: "a.php", line: 2 }]);
    expect(score.falseNegatives).toEqual([]);
    expect(score.explained.map(entry => [entry.side, entry.point.line, entry.divergence.kind])).toEqual([
      ["fp", 1, "upstream-wrong"],
      ["fn", 5, "rerail-unknown"],
    ]);
  });

  it("counts an explained miss against recall unless the upstream finding is wrong", () => {
    const expected = [
      { path: "a.php", line: 1 },
      { path: "a.php", line: 2 },
      { path: "a.php", line: 3 },
    ];
    const actual = [{ path: "a.php", line: 1 }];
    expect(precisionRecall(scoreSource(expected, actual, "line", [divergence("rerail-unknown", 2), divergence("upstream-wrong", 3)]))).toEqual({
      precision: 1,
      recall: 0.5,
    });
  });
});

describe("scoreSamples", () => {
  const divergence = (kind: Divergence["kind"], path: string, line: number): Divergence => ({ rule: "r", source: "samples", path, line, kind, evidence: "checked by hand against the sample" });

  it("catches a label by its line alone; any report on a clean sample is an FP; an unlabelled line of a labelled sample is reviewed", () => {
    const score = scoreSamples(
      [
        { path: "s/a.php", lines: [2, 2, 4] },
        { path: "s/b.php", lines: [] },
      ],
      [
        { path: "s/a.php", line: 2 },
        { path: "s/a.php", line: 7 },
        { path: "s/b.php", line: 1 },
        { path: "s/b.php", line: 1 },
      ],
      [],
    );
    expect(score.tp).toBe(2);
    expect(score.falseNegatives).toEqual([{ path: "s/a.php", line: 4 }]);
    expect(score.falsePositives).toEqual([
      { path: "s/b.php", line: 1 },
      { path: "s/b.php", line: 1 },
    ]);
    expect(score.reviewed).toEqual([{ path: "s/a.php", line: 7 }]);
  });

  it("explains clean-sample findings and missed labels by divergence; rerail-unknown never excuses a finding", () => {
    const score = scoreSamples(
      [
        { path: "s/a.php", lines: [3] },
        { path: "s/b.php", lines: [] },
      ],
      [
        { path: "s/b.php", line: 1 },
        { path: "s/b.php", line: 2 },
      ],
      [divergence("rerail-unknown", "s/a.php", 3), divergence("upstream-wrong", "s/b.php", 1), divergence("rerail-unknown", "s/b.php", 2)],
    );
    expect(score.falseNegatives).toEqual([]);
    expect(score.falsePositives).toEqual([{ path: "s/b.php", line: 2 }]);
    expect(score.explained.map(entry => [entry.side, entry.point.path, entry.point.line])).toEqual([
      ["fn", "s/a.php", 3],
      ["fp", "s/b.php", 1],
    ]);
  });
});
