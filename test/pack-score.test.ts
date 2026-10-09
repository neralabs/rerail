/**
 * Check 4 of pack validation beyond the declared match (docs/pack-format.md, "The validation contract"): ports:span,
 * ports:unknown-misses and ports:evidence, scored on the fixtures as V1's Gate 5 scored upstream tests.
 */
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { loadPack, type Divergence, type LoadedPack } from "../conformance/languages/pack.js";
import { reportFailures, runPack, type PackRun } from "../conformance/languages/runner.js";
import { comparePort, type PortFixtures, type Position } from "../conformance/languages/score.js";
import { validationReportSchema } from "../conformance/validation-report.js";

const SAMPLE = fileURLToPath(new URL("./fixtures/language-conformance/sample-php/", import.meta.url));
const DATE = "2026-01-01";

const at = (path: string, line: number, column: number, endLine: number, endColumn: number): Position => ({ path, line, column, endLine, endColumn });
const divergence = (kind: Divergence["kind"], path: string, line: number): Divergence => ({ rule: "r", path, line, kind, evidence: "checked against the upstream runtime" });
const ALL = ["line", "column", "endLine", "endColumn"] as const;
const port = (input: Partial<PortFixtures> & Pick<PortFixtures, "expected" | "actual">): PortFixtures => ({
  match: "line+column",
  parts: ALL,
  divergences: [],
  units: [...new Set([...input.expected, ...input.actual].map(position => position.path))].map(path => [path]),
  failures: [],
  ...input,
});

describe("comparePort", () => {
  it("files a start-matched pair that differs at its end under ports:span, and unpaired starts under ports:points", () => {
    const result = comparePort(
      port({
        expected: [at("a.py", 1, 1, 1, 5), at("a.py", 2, 1, 2, 4), at("a.py", 3, 1, 3, 2)],
        actual: [at("a.py", 1, 1, 1, 5), at("a.py", 2, 1, 2, 9), at("a.py", 4, 1, 4, 2)],
      }),
    );
    expect(result.span).toEqual([{ expected: at("a.py", 2, 1, 2, 4), actual: at("a.py", 2, 1, 2, 9) }]);
    expect([result.pointsMatched, result.matched]).toEqual([2, 1]);
    expect([result.missing, result.extra]).toEqual([
      [at("a.py", 2, 1, 2, 4), at("a.py", 3, 1, 3, 2)],
      [at("a.py", 2, 1, 2, 9), at("a.py", 4, 1, 4, 2)],
    ]);
    // Line 3's miss and line 4's claim are ports:points' FN and FP; the pack's unit disagrees, so there is no evidence.
    expect([result.agreeing, result.evidence]).toEqual([0, 0]);
  });

  it("pairs within equal starts once every exact pair is taken, so one cause fails one sub-check", () => {
    const result = comparePort(
      port({
        expected: [at("a.py", 1, 1, 1, 5), at("a.py", 1, 1, 1, 9)],
        actual: [at("a.py", 1, 1, 1, 9), at("a.py", 1, 1, 1, 7), at("a.py", 1, 1, 1, 8)],
      }),
    );
    // Two starts match two of three: one pair is exact, one differs at its end, and the third claim is a points FP.
    expect(result.pointsMatched).toBe(2);
    expect(result.span).toEqual([{ expected: at("a.py", 1, 1, 1, 5), actual: at("a.py", 1, 1, 1, 7) }]);
    expect(result.extra).toEqual([at("a.py", 1, 1, 1, 7), at("a.py", 1, 1, 1, 8)]);
  });

  it("compares every part the recording carries, a finer one than the declared match included", () => {
    const expected = [{ path: "a.php", line: 2, column: 18 }];
    const actual = [at("a.php", 2, 17, 2, 30)];
    const byLine = comparePort(port({ expected, actual, match: "line", parts: ["line", "column"] }));
    expect([byLine.pointsMatched, byLine.span]).toEqual([1, [{ expected: expected[0], actual: { path: "a.php", line: 2, column: 17 } }]]);
    // Recorded at the line only, the ends Rerail reports are never compared.
    const lineOnly = comparePort(port({ expected: [{ path: "a.php", line: 2 }], actual, match: "line", parts: ["line"] }));
    expect([lineOnly.span, lineOnly.matched, lineOnly.evidence]).toEqual([[], 1, 1]);
  });

  it("compares Bandit's end column on its own, wherever Rerail's end line falls", () => {
    const expected = [{ path: "a.py", line: 1, column: 5, endColumn: 4 }];
    const result = comparePort(port({ expected, actual: [at("a.py", 1, 5, 3, 4)], parts: ["line", "column", "endColumn"] }));
    expect([result.matched, result.span, result.evidence]).toEqual([1, [], 1]);
    const off = comparePort(port({ expected, actual: [at("a.py", 1, 5, 1, 5)], parts: ["line", "column", "endColumn"] }));
    expect(off.span).toEqual([{ expected: expected[0], actual: { path: "a.py", line: 1, column: 5, endColumn: 5 } }]);
  });

  it("excuses a span difference as points are excused: never a Rerail finding by rerail-unknown", () => {
    const expected = [at("a.py", 1, 1, 1, 5), at("a.py", 2, 1, 2, 5)];
    const actual = [at("a.py", 1, 1, 1, 6), at("a.py", 2, 1, 2, 6)];
    const result = comparePort(port({ expected, actual, divergences: [divergence("not-comparable", "a.py", 1), divergence("rerail-unknown", "a.py", 2)] }));
    expect(result.span).toEqual([{ expected: expected[1], actual: actual[1] }]);
    // Both sides of line 1 and the miss on line 2 are explained; Rerail's claim on line 2 is not.
    expect(result.explained).toBe(3);
  });

  it("fails ports:unknown-misses on a miss that only a rerail-unknown divergence explains", () => {
    const expected = [at("a.js", 1, 3, 1, 9), at("a.js", 2, 3, 2, 9), at("a.js", 3, 3, 3, 9)];
    const result = comparePort(
      port({
        expected,
        actual: [],
        divergences: [divergence("rerail-unknown", "a.js", 1), divergence("rerail-unknown", "a.js", 2), divergence("upstream-wrong", "a.js", 2)],
      }),
    );
    // Line 2's miss is also upstream's error, and line 3's is no divergence's: ports:points fails it.
    expect(result.unknownMisses).toEqual([{ path: "a.js", line: 1, column: 3 }]);
  });

  it("counts as evidence only the upstream findings in units that agree completely", () => {
    const result = comparePort(
      port({
        expected: [at("a.py", 1, 1, 1, 5), at("a.py", 2, 1, 2, 5), at("b.py", 1, 1, 1, 5), at("c.py", 1, 1, 1, 5)],
        actual: [at("a.py", 1, 1, 1, 5), at("a.py", 2, 1, 2, 5), at("b.py", 1, 1, 1, 6), at("c.py", 1, 1, 1, 5), at("d.py", 1, 1, 1, 5)],
        units: [["a.py"], ["b.py"], ["c.py", "c2.py"], ["d.py"], ["e.py"]],
        // A failure without a path is the unit's; a source-partial one never reaches the scorer.
        failures: [{ unit: 2, text: "source-incomplete: file has syntax errors" }],
      }),
    );
    // a.py agrees, b.py differs at an end, c.py's unit has a failure, d.py has a claim upstream does not make, e.py
    // agrees with no finding.
    expect([result.agreeing, result.evidence]).toEqual([2, 2]);
    expect(result.differences).toEqual([
      { path: "b.py", missing: [{ line: 1, column: 1, endLine: 1, endColumn: 5 }], extra: [{ line: 1, column: 1, endLine: 1, endColumn: 6 }], failures: [] },
      { path: "c.py", missing: [], extra: [], failures: ["source-incomplete: file has syntax errors"] },
      { path: "d.py", missing: [], extra: [{ line: 1, column: 1, endLine: 1, endColumn: 5 }], failures: [] },
    ]);
    expect(comparePort(port({ expected: [], actual: [], units: [["a.py"]] })).evidence).toBe(0);
  });
});

describe("runPack's port scores", () => {
  const fixturesOnly = (loaded: LoadedPack): Promise<PackRun> => runPack(loaded, { fixtures: true, corpora: [], date: DATE });
  const failures = (run: PackRun) => Object.fromEntries([...run.ports].map(([id, port]) => [id, port.failures]));
  const passing = { "ports:points": [], "ports:span": [], "ports:unknown-misses": [], "ports:evidence": [] };

  /** The sample pack in a scratch directory, its phpcs fixtures recording rewritten by `edit`. */
  async function withRecording<T>(edit: (recording: { source: { files: string[] }; findings: Position[] }, fixtures: string) => void, use: (loaded: LoadedPack) => Promise<T>): Promise<T> {
    const directory = mkdtempSync(join(tmpdir(), "pack-score-"));
    try {
      cpSync(SAMPLE, directory, { recursive: true });
      const path = join(directory, "upstream", "phpcs", "fixtures.expected.json");
      const recording = JSON.parse(readFileSync(path, "utf8")) as { source: { files: string[] }; findings: Position[] };
      edit(recording, join(directory, "upstream", "phpcs", "fixtures"));
      writeFileSync(path, JSON.stringify(recording));
      return await use(loadPack(directory));
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }

  it("scores the sample's ports on the fixtures, as the validation report takes them, with no date or timing", async () => {
    const run = await fixturesOnly(loadPack(SAMPLE));
    expect(failures(run)).toEqual({
      "forbidden-functions": passing,
      "forbidden-functions-resolved": {
        "ports:points": [],
        "ports:span": [],
        // The call in broken.php, which the resolved port skips for the file's syntax errors.
        "ports:unknown-misses": ['forbidden-functions-resolved (fixtures): 1 upstream finding(s) missed and explained only by a rerail-unknown divergence, first {"path":"broken.php","line":3,"column":6}'],
        "ports:evidence": [
          "forbidden-functions-resolved (fixtures): no upstream finding is reproduced in a unit where the port agrees completely (1 of 3 unit(s) agree; upstream reports 6 finding(s))",
        ],
      },
    });
    expect(run.ports.get("forbidden-functions-resolved")?.working).toEqual({
      parts: ["line", "column"],
      units: 3,
      expected: 6,
      actual: 4,
      pointsMatched: 4,
      matched: 4,
      missing: 2,
      extra: 0,
      explained: 2,
      evidence: 0,
      differences: [
        { path: "broken.php", missing: [{ line: 3, column: 6 }], extra: [], failures: ["source-incomplete: file has syntax errors"] },
        { path: "forbidden.php", missing: [{ line: 8, column: 1 }], extra: [], failures: [] },
      ],
      sources: [{ source: "fixtures", tp: 4, fp: 0, fn: 0, explained: 2, engineFailures: 0 }],
    });
    const portsSchema = validationReportSchema.shape.rules.element.shape.ports;
    for (const port of run.ports.values()) expect(portsSchema.safeParse(port.working).success).toBe(true);
    const again = await fixturesOnly(loadPack(SAMPLE));
    expect(JSON.stringify([[...again.ports], again.unowned])).toBe(JSON.stringify([[...run.ports], run.unowned]));
  });

  it("reads the engine's ends as exclusive code-point columns, on the line the end falls", async () => {
    // Every finding is a whole call; forbidden.php line 7 starts with a two-byte character, and multiline.php's call
    // ends two lines down.
    const ends: Record<string, [number, number]> = {
      "broken.php:3:6": [3, 20],
      "forbidden.php:4:6": [4, 20],
      "forbidden.php:7:6": [7, 21],
      "forbidden.php:7:28": [7, 42],
      "forbidden.php:8:1": [8, 14],
      "forbidden.php:9:49": [9, 59],
      "multiline.php:3:6": [5, 2],
    };
    const withEnds = (wrong?: string) => (recording: { source: { files: string[] }; findings: Position[] }, fixtures: string) => {
      writeFileSync(join(fixtures, "multiline.php"), "<?php\n\n$m = sizeof(\n  $é\n);\n");
      recording.source.files.push("multiline.php");
      recording.findings.push({ ...recording.findings[0]!, path: "multiline.php", line: 3, column: 6 });
      for (const finding of recording.findings) {
        const key = `${finding.path}:${finding.line}:${finding.column}`;
        const [endLine, endColumn] = ends[key]!;
        Object.assign(finding, { endLine, endColumn: key === wrong ? endColumn - 1 : endColumn });
      }
    };
    await withRecording(withEnds(), async loaded => {
      const run = await fixturesOnly(loaded);
      expect(failures(run)["forbidden-functions"]).toEqual(passing);
      expect(run.ports.get("forbidden-functions")?.working).toMatchObject({ parts: ["line", "column", "endLine", "endColumn"], expected: 7, matched: 7, evidence: 7 });
    });
    await withRecording(withEnds("forbidden.php:7:28"), async loaded => {
      const run = await fixturesOnly(loaded);
      // The T4 bar compares starts at the declared match, so it still passes; ports:span fails the end.
      expect(reportFailures(run)).toEqual([]);
      expect(failures(run)["forbidden-functions"]).toEqual({
        ...passing,
        "ports:span": [
          'forbidden-functions (fixtures): 1 finding(s) agree at line+column but differ at endLine, endColumn, first upstream {"path":"forbidden.php","line":7,"column":28,"endLine":7,"endColumn":41}, Rerail {"path":"forbidden.php","line":7,"column":28,"endLine":7,"endColumn":42}',
        ],
      });
      expect(run.ports.get("forbidden-functions")?.working).toMatchObject({ pointsMatched: 7, matched: 6, missing: 1, extra: 1, evidence: 2 });
    });
  });

  it("gives each port its share of the run's failures, and the tool's own errors to no port", async () => {
    const sample = loadPack(SAMPLE);
    const stale: Divergence = { rule: "forbidden-functions", source: "fixtures", path: "forbidden.php", line: 4, kind: "upstream-wrong", evidence: "documented when Rerail missed this call" };
    await withRecording(
      recording => recording.source.files.push("notes.txt"),
      async loaded => {
        const run = await fixturesOnly({ ...loaded, divergences: [...sample.divergences.filter(divergence => divergence.path !== "broken.php"), stale] });
        expect(run.unowned).toEqual(["fixtures phpcs: notes.txt is not a php source by the host's routing"]);
        expect(run.ports.get("forbidden-functions")?.failures["ports:points"]).toEqual(["forbidden-functions: stale divergence forbidden.php:4 (upstream-wrong) explains nothing"]);
        expect(run.ports.get("forbidden-functions-resolved")?.failures["ports:points"]).toEqual([
          'forbidden-functions-resolved (fixtures): 1 unexplained FN, first {"path":"broken.php","line":3,"column":6}',
        ]);
        expect([...[...run.ports.values()].flatMap(port => port.failures["ports:points"]), ...run.unowned].sort()).toEqual(reportFailures(run).sort());
      },
    );
  });

  it("does not run the fixture sub-checks of a port the fixtures did not score", async () => {
    const corpusOnly = await runPack(loadPack(SAMPLE), { fixtures: false, corpora: ["mini"], corpusRoot: `${SAMPLE}corpora`, date: DATE });
    const notRun = ["not run: the fixtures were not scored"];
    expect(corpusOnly.ports.get("forbidden-functions")).toEqual({
      failures: { "ports:points": [], "ports:span": notRun, "ports:unknown-misses": notRun, "ports:evidence": notRun },
      working: null,
    });
    await withRecording(
      recording => Object.assign(recording.source, { rules: ["Generic.PHP.ForbiddenFunctions.Found"] }),
      async loaded => {
        const run = await fixturesOnly(loaded);
        const error = "forbidden-functions (fixtures): the phpcs recording does not cover Generic.PHP.ForbiddenFunctions.FoundWithAlternative";
        expect(run.ports.get("forbidden-functions")?.failures).toEqual({
          "ports:points": [error],
          "ports:span": [`not run: ${error}`],
          "ports:unknown-misses": [`not run: ${error}`],
          "ports:evidence": [`not run: ${error}`],
        });
      },
    );
  });
});
