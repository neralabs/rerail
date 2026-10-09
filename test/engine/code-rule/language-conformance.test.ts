/**
 * The language-pack conformance standard's CI tier (docs/design/rerail-language-conformance.md): every pack under
 * conformance/languages/ scores its ported rules against the upstream rule fixtures with no unexplained FP or FN,
 * through the production workspace path. The sample pack pins the runner's scoring end to end.
 */
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { listPacks, loadPack, packDirectory, packSchema, type CapabilityExemption, type Divergence } from "../conformance/languages/pack.js";
import { reportFailures, runPack } from "../conformance/languages/runner.js";
import { precisionRecall, scoreSource } from "../conformance/languages/score.js";

const SAMPLE = fileURLToPath(new URL("./fixtures/language-conformance/sample-php/", import.meta.url));
const DATE = "2026-01-01";

describe.each(listPacks())("language pack %s", pack => {
  it("passes the fixture tier", async () => {
    const report = await runPack(loadPack(packDirectory(pack)), { fixtures: true, corpora: [], date: DATE });
    expect(reportFailures(report)).toEqual([]);
  }, 300_000);
});

describe("language conformance runner", () => {
  it("scores the sample ports against PHP_CodeSniffer's recordings on fixtures and corpus", async () => {
    const report = await runPack(loadPack(SAMPLE), { fixtures: true, corpora: ["mini"], corpusRoot: `${SAMPLE}corpora`, date: DATE });
    expect(report.rules.map(({ rule, source, tp, fp, fn, precision, recall }) => ({ rule, source, tp, fp, fn, precision, recall }))).toEqual([
      // The regex port runs around broken.php's syntax error and finds its sizeof call.
      { rule: "forbidden-functions", source: "fixtures", tp: 6, fp: 0, fn: 0, precision: 1, recall: 1 },
      // Explained misses lower recall: broken.php's call (rerail-unknown) and the namespaced delete (not-comparable).
      { rule: "forbidden-functions-resolved", source: "fixtures", tp: 4, fp: 0, fn: 0, precision: 1, recall: 4 / 6 },
      { rule: "forbidden-functions", source: "corpus:mini", tp: 1, fp: 0, fn: 0, precision: 1, recall: 1 },
      { rule: "forbidden-functions-resolved", source: "corpus:mini", tp: 1, fp: 0, fn: 0, precision: 1, recall: 1 },
    ]);
    // The Blade template is routed to Blade, not PHP, and is never evaluated by a PHP pack.
    expect(report.sources.find(source => source.source === "corpus:mini")?.files).toBe(1);
    expect(report.pass).toBe(true);
  });

  it("attributes a clause skipped for the file's syntax errors to T1, not to engine failures", async () => {
    const sample = loadPack(SAMPLE);
    const report = await runPack(sample, { fixtures: true, corpora: [], date: DATE });
    const broken = { count: 1, files: [{ path: "broken.php", line: 6, column: 1 }] };
    expect(report.sources[0]?.syntaxErrorFiles).toEqual(broken);
    expect(report.rules.map(({ rule, engineFailures, partialFiles, syntaxErrorFiles }) => ({ rule, engineFailures, partialFiles, syntaxErrorFiles }))).toEqual([
      // Sound under errors: it ran around the error region (`source-partial`).
      { rule: "forbidden-functions", engineFailures: 0, partialFiles: 1, syntaxErrorFiles: { count: 0, files: [] } },
      // Name resolution is unsound under errors: it skipped the file (`source-incomplete`).
      { rule: "forbidden-functions-resolved", engineFailures: 0, partialFiles: 0, syntaxErrorFiles: broken },
    ]);
    // The skipped file's upstream finding is still a miss: without its divergence it fails the fixture tier.
    const undocumented = await runPack({ ...sample, divergences: sample.divergences.filter(divergence => divergence.path !== "broken.php") }, { fixtures: true, corpora: [], date: DATE });
    expect(reportFailures(undocumented)).toEqual(['forbidden-functions-resolved (fixtures): 1 unexplained FN, first {"path":"broken.php","line":3,"column":6}']);
  });

  it("fails on a divergence that no longer explains anything", async () => {
    const sample = loadPack(SAMPLE);
    const stale: Divergence = { rule: "forbidden-functions", source: "fixtures", path: "forbidden.php", line: 4, kind: "upstream-wrong", evidence: "documented when Rerail missed this call" };
    const loaded = { ...sample, divergences: [...sample.divergences, stale] };
    const fixturesOnly = await runPack(loaded, { fixtures: true, corpora: [], date: DATE });
    expect(fixturesOnly.pass).toBe(false);
    expect(reportFailures(fixturesOnly)).toEqual(["forbidden-functions: stale divergence forbidden.php:4 (upstream-wrong) explains nothing"]);
    // A divergence for every source is judged only once every source has been scored.
    const { source: _fixtures, ...everySource } = stale;
    expect((await runPack({ ...sample, divergences: [...sample.divergences, everySource] }, { fixtures: true, corpora: [], date: DATE })).staleDivergences).toEqual([]);
  });

  it("scores no finding, either side's, on corpus lines the tool did not analyse", async () => {
    const sample = loadPack(SAMPLE);
    const oracles = mkdtempSync(join(tmpdir(), "language-conformance-"));
    try {
      cpSync(sample.oraclesDirectory, oracles, { recursive: true });
      const path = join(oracles, "phpcs", "corpus-mini.expected.json");
      const recording = JSON.parse(readFileSync(path, "utf8")) as { source: Record<string, unknown>; findings: unknown[] };
      // The tool stopped analysing `of()` (lines 7-10), so it is silent on the `sizeof` call Rerail finds on line 9.
      const silent = (unanalysed: unknown[]) => writeFileSync(path, JSON.stringify({ ...recording, source: { ...recording.source, unanalysed }, findings: [] }));
      const run = async () => (await runPack({ ...sample, oraclesDirectory: oracles }, { fixtures: false, corpora: ["mini"], corpusRoot: `${SAMPLE}corpora`, date: DATE })).rules;
      silent([]);
      expect((await run()).map(({ rule, tp, fp, unanalysed }) => ({ rule, tp, fp, unanalysed }))).toEqual([
        { rule: "forbidden-functions", tp: 0, fp: 1, unanalysed: 0 },
        { rule: "forbidden-functions-resolved", tp: 0, fp: 1, unanalysed: 0 },
      ]);
      silent([{ path: "src/Size.php", startLine: 7, endLine: 10 }]);
      expect((await run()).map(({ rule, tp, fp, fn, unanalysed }) => ({ rule, tp, fp, fn, unanalysed }))).toEqual([
        { rule: "forbidden-functions", tp: 0, fp: 0, fn: 0, unanalysed: 1 },
        { rule: "forbidden-functions-resolved", tp: 0, fp: 0, fn: 0, unanalysed: 1 },
      ]);
      // A region elsewhere in the file leaves the finding scored.
      silent([{ path: "src/Size.php", startLine: 1, endLine: 8 }]);
      expect((await run()).map(({ fp }) => fp)).toEqual([1, 1]);
    } finally {
      rmSync(oracles, { recursive: true, force: true });
    }
  });

  describe("capability exemptions", () => {
    const raw = JSON.parse(readFileSync(`${SAMPLE}conformance.json`, "utf8")) as Record<string, unknown>;
    const exemption = (capability: string, coveredBy = ["T2: fixtures/php/calls/", "T3: benchmark_php_calls_corpus"]): CapabilityExemption => ({
      capability,
      reason: "no pinned linter reads callees transitively",
      evidence: "PHPStan 2.2.16, Psalm 6.19.1 and PHPCS 4.0.4 rules read only the direct callee's declaration",
      coveredBy,
    });
    const withExemption = (capabilities: string[], exemptions: CapabilityExemption[]) =>
      ({ ...loadPack(SAMPLE), pack: packSchema.parse({ ...raw, capabilities, exemptCapabilities: exemptions }) });

    it("covers a declared capability no rule exercises", async () => {
      const report = await runPack(withExemption(["syntax", "scope", "calls"], [exemption("calls")]), { fixtures: true, corpora: [], date: DATE });
      expect(report.coverage.uncovered).toEqual([]);
      expect(report.coverage.exempt.map(entry => entry.capability)).toEqual(["calls"]);
      expect(report.pass).toBe(true);
    });

    it("is stale once a rule exercises the capability", async () => {
      const report = await runPack(withExemption(["syntax", "scope"], [exemption("syntax")]), { fixtures: true, corpora: [], date: DATE });
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
