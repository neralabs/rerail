/**
 * The conformance runner's real-code samples (`runSamples`, `scoreSamples` and `reportFailures`,
 * conformance/languages/), and the conformance checks on samples: samples are judged by intent, line only.
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
import { conformanceChecks, conformanceOf } from "../../conformance/languages/checks/conformance.js";
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

/** The conformance checks assert-used fails, from the report the gate would read; stays-silent is judged below. */
const failingChecks = (report: T4Report): string[] =>
  conformanceChecks(conformanceOf(loadPack(directory), report), "assert-used")
    .filter(check => !check.ok && check.id !== "stays-silent")
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

/**
 * Gives assert-used a second clause: every pass statement of an app.py needs one in a ctx.py beside it
 * (`require_each` over evidence). A ctx.py the grammar cannot read leaves app.py's pass statements unknown, a
 * parse-induced `source-incomplete` ("evidence is incomplete in …") on app.py.
 */
const addEvidenceClause = (): void =>
  editJson(join(directory, "rules", "assert-used.json"), rule => {
    const relation = (as: string, file: string) => ({
      evidence: { as, paths: { include: [`**/${file}`] }, query: { ast: { language: "python", rule: { kind: "pass_statement" } } }, project: { name: "file.name" } },
    });
    const evidence = { id: "pass-has-evidence", require_each: { subject: relation("subject", "app.py"), evidence: relation("evidence", "ctx.py") }, violation: { anchor: "witness.span", code: "EX101", message: "examplelint EX101" } };
    return { ...rule, clauses: [...(rule.clauses as unknown[]), evidence] };
  });

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

  it("judges a sample the grammar reads in part: it fails the rule only through a label the partial read hides", async () => {
    const caught = await score([{ name: "unread", path: "app.py", text: UNREADABLE, expected: { "assert-used": [] } }], { fixtures: true });
    expect(samplesRow(caught.report)).toMatchObject({ fp: 0, fn: 0, engineFailures: 0, syntaxErrorFiles: { count: 1 } });
    expect(reportFailures(caught.report)).toEqual([]);
    expect(failingChecks(caught.report)).toEqual([]);
    const missed = await score([{ name: "unread", path: "app.py", text: UNREADABLE, expected: { "assert-used": [1] } }], { fixtures: true });
    expect(samplesRow(missed.report)).toMatchObject({ fn: 1, engineFailures: 0, syntaxErrorFiles: { count: 1 } });
    expect(failingChecks(missed.report)).toEqual(["samples-catch-labelled"]);
  });

  it("does not fail on an engine failure in a context file, which no label judges", async () => {
    // More asserts than the engine's witness limit (10,000) in a context file: the rule fails there, not on app.py.
    const context = { "ctx.py": "assert ready\n".repeat(10_001) };
    const { report } = await score([{ name: "context", path: "app.py", text: "assert ready\n", expected: { "assert-used": [1] }, context }], { fixtures: true });
    expect(samplesRow(report)).toMatchObject({ tp: 1, fn: 0, engineFailures: 0, contextFailures: 1 });
    expect(reportFailures(report)).toEqual([]);
    expect(failingChecks(report)).toEqual([]);
  });

  it("judges a sample a parse-induced source-incomplete reads in part, passing when every label is caught", async () => {
    addEvidenceClause();
    const context = { "ctx.py": UNREADABLE };
    const { report } = await score([{ name: "evidence", path: "app.py", text: "assert ready\npass\n", expected: { "assert-used": [1] }, context }], { fixtures: true });
    expect(samplesRow(report)).toMatchObject({ tp: 1, fn: 0, engineFailures: 0, partialFiles: 1 });
    expect(reportFailures(report)).toEqual([]);
    expect(failingChecks(report)).toEqual([]);
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

/** Sets examplelint's fixture units' origins (`source.units`), and adds `findings` to its expected findings. */
const setUnits = (units: Record<string, { rules: string[]; renamedFrom?: string }>, files: string[] = [], findings: unknown[] = []): void =>
  editJson(join(directory, "upstream", "examplelint", "fixtures.expected.json"), expected => {
    const source = expected.source as Record<string, unknown>;
    return { ...expected, source: { ...source, files: [...(source.files as string[]), ...files], units }, findings: [...(expected.findings as unknown[]), ...findings] };
  });

describe("stays-silent", () => {
  const staysSilent = (report: T4Report) => conformanceChecks(conformanceOf(loadPack(directory), report), "assert-used").find(check => check.id === "stays-silent")!;
  const fixturesRow = (report: T4Report): RuleRow => report.rules.find(row => row.source === "fixtures")!;
  const clean: HandMadeSample = { name: "clean", path: "app.py", text: "ready = True\n", expected: { "assert-used": [] } };

  it("is not met by fixture units of unknown origin or of other rules, however silent; a silent clean sample meets it", async () => {
    // clean.py is a fixture unit examplelint reports nothing on and the rule is silent on, but no origin names the
    // rule's upstream rule, so it proves nothing about the rule.
    const unknown = await runPack(loadPack(directory), { fixtures: true, samples: false, date: DATE });
    expect(fixturesRow(unknown)).toMatchObject({ ownUnits: 0, silentOwnUnits: 0 });
    expect(staysSilent(unknown)).toMatchObject({ ok: false });
    setUnits({ "clean.py": { rules: ["EX202"] } });
    expect(staysSilent(await runPack(loadPack(directory), { fixtures: true, samples: false, date: DATE }))).toMatchObject({ ok: false });
    const { report } = await score([clean], { fixtures: true });
    expect(samplesRows(report)).toEqual([expect.objectContaining({ cleanSamples: 1, silentCleanSamples: 1 })]);
    expect(staysSilent(report)).toEqual({ id: "stays-silent", ok: true, detail: "silent where it should be: 0 of 0 own upstream unit(s), 1 clean sample(s)" });
  });

  it("is met by the rule's own units upstream and the rule are silent on, and not by its own units upstream reports", async () => {
    setUnits({ "asserts.py": { rules: ["EX101"] }, "clean.py": { rules: ["EX101"] } });
    const report = await runPack(loadPack(directory), { fixtures: true, samples: false, date: DATE });
    expect(fixturesRow(report)).toMatchObject({ ownUnits: 2, silentOwnUnits: 1, upstreamFindings: 2 });
    expect(staysSilent(report)).toEqual({ id: "stays-silent", ok: true, detail: "silent where it should be: 1 of 2 own upstream unit(s), 0 clean sample(s)" });
  });

  it("is not met by a clean sample the engine cannot read", async () => {
    const { report } = await score([{ ...clean, text: UNREADABLE }], { fixtures: true });
    expect(staysSilent(report).ok).toBe(false);
  });

  it("is not met by a clean sample read in part, nor by one a parse-induced source-incomplete leaves unknown", async () => {
    const partial = await score([{ ...clean, text: "ready = True\nx = (\n" }], { fixtures: true });
    expect(samplesRow(partial.report)).toMatchObject({ cleanSamples: 1, silentCleanSamples: 0, partialFiles: 1 });
    expect(staysSilent(partial.report).ok).toBe(false);
    rmSync(tree, { recursive: true, force: true });
    addEvidenceClause();
    const incomplete = await score([{ ...clean, text: "ready = True\npass\n", context: { "ctx.py": UNREADABLE } }], { fixtures: true });
    expect(samplesRow(incomplete.report)).toMatchObject({ cleanSamples: 1, silentCleanSamples: 0, engineFailures: 0, partialFiles: 1 });
    expect(staysSilent(incomplete.report).ok).toBe(false);
  });

  it("is not met by a clean sample the rule's `file` condition or scope rules out, where the rule never looks", async () => {
    const restrict = (edit: (rule: Record<string, unknown>) => Record<string, unknown>): void => editJson(join(directory, "rules", "assert-used.json"), edit);
    restrict(rule => ({
      ...rule,
      clauses: (rule.clauses as { forbid: { ast: object } }[]).map(clause => ({ ...clause, forbid: { ast: { ...clause.forbid.ast, file: { path: { regex: "(^|/)tests/" } } } } })),
    }));
    const outside = await score([clean], { fixtures: true });
    expect(samplesRow(outside.report)).toMatchObject({ cleanSamples: 1, silentCleanSamples: 0 });
    expect(staysSilent(outside.report).ok).toBe(false);
    rmSync(tree, { recursive: true, force: true });
    const inside = await score([{ ...clean, path: "tests/app.py" }], { fixtures: true });
    expect(samplesRow(inside.report)).toMatchObject({ cleanSamples: 1, silentCleanSamples: 1 });
    expect(staysSilent(inside.report).ok).toBe(true);
    rmSync(tree, { recursive: true, force: true });
    restrict(rule => ({ ...rule, scope: { include: ["**/*"], exclude: ["**/tests/**"] } }));
    const excluded = await score([{ ...clean, path: "tests/app.py" }], { fixtures: true });
    expect(samplesRow(excluded.report)).toMatchObject({ cleanSamples: 1, silentCleanSamples: 0 });
  });
});

describe("rename variants", () => {
  /** renamed/asserts.py: asserts.py renamed, labelled by examplelint as it labels the original (the nested assert skipped). */
  const addVariant = (): void => {
    const fixtures = join(directory, "upstream", "examplelint", "fixtures");
    mkdirSync(join(fixtures, "renamed"), { recursive: true });
    writeFileSync(join(fixtures, "renamed", "asserts.py"), "assert q\nz = 1\nassert w\nif z:\n    assert e\n");
    setUnits(
      { "asserts.py": { rules: ["EX101"] }, "renamed/asserts.py": { rules: ["EX101"], renamedFrom: "asserts.py" } },
      ["renamed/asserts.py"],
      [1, 3].map(line => ({ rule: "EX101", path: "renamed/asserts.py", line, column: 1 })),
    );
  };
  const fixturesRow = (report: T4Report): RuleRow => report.rules.find(row => row.source === "fixtures")!;

  it("are scored apart from the originals and never count as the rule's own units", async () => {
    const before = fixturesRow(await runPack(loadPack(directory), { fixtures: true, samples: false, date: DATE }));
    expect(before.renamed).toBeNull();
    addVariant();
    const report = await runPack(loadPack(directory), { fixtures: true, samples: false, date: DATE });
    const row = fixturesRow(report);
    expect(row).toMatchObject({ tp: before.tp, fp: before.fp, fn: before.fn, upstreamFindings: before.upstreamFindings, ownUnits: 1 });
    // The original's divergence names asserts.py:5, so it explains nothing on the variant: holds-under-rename fails.
    expect(row.renamed).toEqual({ units: 1, tp: 2, fp: 1, fn: 0, falsePositives: [{ path: "renamed/asserts.py", line: 5, column: 5 }], falseNegatives: [], engineFailures: 0, incomplete: 0, renamedAway: 0 });
    expect(reportFailures(report)).toEqual([`assert-used (fixtures): 1 unexplained FP on rename variants, first {"path":"renamed/asserts.py","line":5,"column":5}`]);
    expect(failingChecks(report)).toEqual(["holds-under-rename"]);
  });

  it("hold once a divergence explains the variant's disagreement", async () => {
    addVariant();
    writeDivergences([{ rule: "assert-used", source: "fixtures", path: "renamed/asserts.py", line: 5, kind: "upstream-wrong", evidence: "examplelint skips an assert nested in an if block; line 5 is one" }]);
    const report = await runPack(loadPack(directory), { fixtures: true, samples: false, date: DATE });
    expect(fixturesRow(report).renamed).toMatchObject({ tp: 2, fp: 0, fn: 0 });
    expect(reportFailures(report)).toEqual([]);
    expect(failingChecks(report)).toEqual([]);
  });

  it("fail a shipped rule on an engine failure on a variant upstream is silent on", async () => {
    // More asserts than the engine's witness limit (10,000): the rule fails on the variant, where examplelint (as
    // written here) reports nothing.
    const fixtures = join(directory, "upstream", "examplelint", "fixtures");
    mkdirSync(join(fixtures, "renamed"), { recursive: true });
    writeFileSync(join(fixtures, "renamed", "clean.py"), "assert q\n".repeat(10_001));
    setUnits({ "clean.py": { rules: ["EX101"] }, "renamed/clean.py": { rules: ["EX101"], renamedFrom: "clean.py" } }, ["renamed/clean.py"]);
    const report = await runPack(loadPack(directory), { fixtures: true, samples: false, date: DATE });
    expect(fixturesRow(report).renamed).toMatchObject({ units: 1, fp: 0, fn: 0, engineFailures: 1, incomplete: 0 });
    expect(fixturesRow(report).engineFailures).toBe(0);
    expect(reportFailures(report)).toEqual(["assert-used (fixtures): 1 engine failure(s) on rename variants"]);
    expect(failingChecks(report)).toEqual(["holds-under-rename"]);
  });

  it("count a variant the renaming took every finding of the rule from, without failing: vacuous, reported", async () => {
    // renamed/asserts.py is asserts.py with its asserts gone: examplelint reports the original, not the variant.
    const fixtures = join(directory, "upstream", "examplelint", "fixtures");
    mkdirSync(join(fixtures, "renamed"), { recursive: true });
    writeFileSync(join(fixtures, "renamed", "asserts.py"), "z = 1\n");
    setUnits({ "asserts.py": { rules: ["EX101"] }, "renamed/asserts.py": { rules: ["EX101"], renamedFrom: "asserts.py" } }, ["renamed/asserts.py"]);
    const report = await runPack(loadPack(directory), { fixtures: true, samples: false, date: DATE });
    expect(fixturesRow(report).renamed).toMatchObject({ units: 1, tp: 0, fp: 0, fn: 0, renamedAway: 1 });
    expect(reportFailures(report)).toEqual([]);
    const check = conformanceChecks(conformanceOf(loadPack(directory), report), "assert-used").find(entry => entry.id === "holds-under-rename");
    expect(check).toEqual({ id: "holds-under-rename", ok: true, vacuous: true, detail: "vacuous: no finding of the rule survives on its 1 rename variant unit(s); 1 variant(s) lost every finding of the rule (renamed away)" });
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
