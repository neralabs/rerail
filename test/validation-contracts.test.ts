/**
 * The rule validation layer's contracts: the checks and sub-checks (conformance/checks.ts), finding ends and declared
 * source facts in the conformance schemas (conformance/languages/pack.ts), upstream/<tool>/source.json, scrub.json
 * (conformance/scrub.ts) and the pack validation report (conformance/validation-report.ts). Each schema is shown
 * accepting what it describes and refusing each thing it rules out.
 */
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { z } from "zod";
import {
  PACK_CHECKS,
  RULE_SUB_CHECK_IDS,
  SCRUBBABLE_SUB_CHECK_IDS,
  SUB_CHECK_IDS,
  SUB_CHECK_OUTCOMES,
  SUB_CHECKS,
  type RuleSubCheckId,
  type SubCheckOutcome,
} from "../conformance/checks.js";
import { loadUpstreamSource, packSchema, recordingSchema, upstreamSourceSchema, type UpstreamSource } from "../conformance/languages/pack.js";
import { validatePack } from "../conformance/pack-validation.js";
import { loadScrub, SCRUB_FILE, scrubSchema } from "../conformance/scrub.js";
import { VALIDATION_REPORT_FORMAT, validationReportSchema, type RuleReport, type ValidationReport } from "../conformance/validation-report.js";

const ROOT = fileURLToPath(new URL("../", import.meta.url));
const PACKS = join(ROOT, "packs");
const VALID_PACK = fileURLToPath(new URL("./fixtures/packs/valid/", import.meta.url));

const temporary: string[] = [];
afterEach(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});
/** A new temporary directory holding `name` with `text`. */
const written = (name: string, text: string): string => {
  const directory = mkdtempSync(join(tmpdir(), "rerail-contracts-"));
  temporary.push(directory);
  writeFileSync(join(directory, name), text);
  return directory;
};

const issues = (schema: z.ZodType, value: unknown): string => {
  const parsed = schema.safeParse(value);
  return parsed.success ? "" : z.prettifyError(parsed.error);
};

describe("checks and sub-checks", () => {
  it("numbers the nine checks in order", () => {
    expect(PACK_CHECKS.map(check => `${check.number} ${check.name}`)).toEqual([
      "1 rules",
      "2 proof",
      "3 examples",
      "4 ports",
      "5 package",
      "6 licences",
      "7 provenance",
      "8 generality",
      "9 size",
    ]);
  });

  it("files every sub-check under its check, in check order, each id once", () => {
    expect(new Set(SUB_CHECK_IDS).size).toBe(SUB_CHECK_IDS.length);
    for (const check of SUB_CHECKS) expect(check.id === check.check || check.id.startsWith(`${check.check}:`), check.id).toBe(true);
    const numbers = SUB_CHECKS.map(check => PACK_CHECKS.find(entry => entry.name === check.check)!.number);
    expect(numbers).toEqual([...numbers].sort((left, right) => left - right));
    expect(new Set(SUB_CHECKS.map(check => check.check))).toEqual(new Set(PACK_CHECKS.map(check => check.name)));
  });

  it("gives every rule a result for these sub-checks, in this order", () => {
    expect(RULE_SUB_CHECK_IDS).toEqual([
      "rules",
      "rules:deterministic",
      "rules:in-scope",
      "proof",
      "examples",
      "ports:points",
      "ports:span",
      "ports:unknown-misses",
      "ports:evidence",
      "ports:coverage",
      "provenance:upstream-rule",
      "provenance:stable",
      "provenance:recommended",
      "generality:names",
      "generality:regexes",
      "size:combinatorial-expansion",
      "size:written-out-repetition",
    ]);
  });

  it("never scrubs the language packs' own bar, a pack-level or a tool-level failure", () => {
    expect(SCRUBBABLE_SUB_CHECK_IDS).toEqual([
      "ports:span",
      "ports:unknown-misses",
      "ports:evidence",
      "ports:coverage",
      "provenance:upstream-rule",
      "provenance:stable",
      "provenance:recommended",
      "generality:names",
      "generality:regexes",
      "size:combinatorial-expansion",
      "size:written-out-repetition",
    ]);
    for (const check of SUB_CHECKS) if (check.level !== "rule") expect(check.scrub, check.id).toBeNull();
    expect(SUB_CHECKS.filter(check => check.scrub === "upstream-policy").map(check => check.id)).toEqual(["provenance:stable", "provenance:recommended"]);
  });

  it("needs source facts exactly for checks 7 to 9 and check 4's coverage", () => {
    for (const check of SUB_CHECKS) expect(check.needsSourceFacts, check.id).toBe(["provenance", "generality", "size"].includes(check.check) || check.id === "ports:coverage");
  });

  it("validatePack reports all nine checks", async () => {
    const validation = await validatePack(VALID_PACK, { corpora: false });
    expect(Object.keys(validation.checks)).toEqual(PACK_CHECKS.map(check => check.name));
    expect(validation.notJudged).toEqual({});
    expect(validation.pass).toBe(true);
  });
});

describe("finding ends", () => {
  const recording = (...findings: Record<string, unknown>[]) => ({
    tool: "examplelint",
    version: "1.0.0",
    source: { kind: "fixtures", origin: "https://example.com/examplelint@0000000", rules: ["EX101", "EX102"], files: ["a.py"] },
    findings: findings.map(finding => ({ rule: "EX101", path: "a.py", ...finding })),
  });

  it.each([
    ["a line", { line: 3 }],
    ["a line and column", { line: 3, column: 5 }],
    ["an end column without an end line (Bandit)", { line: 3, column: 5, endColumn: 2 }],
    ["a full span", { line: 3, column: 5, endLine: 4, endColumn: 1 }],
    ["an empty span", { line: 3, column: 5, endLine: 3, endColumn: 5 }],
  ])("accepts %s", (_name, finding) => {
    expect(issues(recordingSchema, recording(finding))).toBe("");
  });

  it.each([
    ["an end line without an end column", { line: 3, column: 5, endLine: 4 }, "endLine needs endColumn"],
    ["an end column without a column", { line: 3, endColumn: 6 }, "endColumn needs column"],
    ["an end line before the line", { line: 3, column: 5, endLine: 2, endColumn: 9 }, "endLine 2 is before line 3"],
    ["an end before the start on one line", { line: 3, column: 5, endLine: 3, endColumn: 4 }, "endColumn 4 is before column 5 on line 3"],
    ["a 0 end column", { line: 3, column: 5, endLine: 3, endColumn: 0 }, "endColumn"],
  ])("refuses %s", (_name, finding, message) => {
    expect(issues(recordingSchema, recording(finding))).toContain(message);
  });

  it("refuses one rule's findings carrying different parts, and allows rules to differ", () => {
    expect(issues(recordingSchema, recording({ line: 1, column: 1 }, { line: 2, column: 1, endLine: 2, endColumn: 4 }))).toContain(
      "EX101: findings carry different parts (line, column / line, column, endLine, endColumn)",
    );
    expect(issues(recordingSchema, recording({ line: 1, column: 1 }, { rule: "EX102", line: 2, column: 1, endLine: 2, endColumn: 4 }))).toBe("");
  });

  it("keeps every committed recording of the packs valid", () => {
    const paths = readdirSync(PACKS).flatMap(pack => {
      const upstream = join(PACKS, pack, "upstream");
      if (!existsSync(upstream)) return [];
      return readdirSync(upstream, { withFileTypes: true })
        .filter(entry => entry.isDirectory())
        .flatMap(tool => readdirSync(join(upstream, tool.name)).filter(name => name.endsWith(".expected.json")).map(name => join(upstream, tool.name, name)));
    });
    expect(paths.length).toBeGreaterThan(0);
    for (const path of paths) expect(issues(recordingSchema, JSON.parse(readFileSync(path, "utf8"))), path).toBe("");
  });
});

describe("conformance.json tools[].recommended", () => {
  const pack = (tool: Record<string, unknown>) => ({ pack: "valid", family: "python", capabilities: ["syntax"], corpora: [], tools: [tool], rules: [] });

  it("is optional and ordered", () => {
    expect(issues(packSchema, pack({ name: "ruff", version: "0.16.9" }))).toBe("");
    expect(packSchema.parse(pack({ name: "eslint", version: "10.11.0", recommended: ["recommended", "all"] })).tools[0]!.recommended).toEqual(["recommended", "all"]);
  });

  it("refuses an empty list and a repeated configuration", () => {
    expect(issues(packSchema, pack({ name: "ruff", version: "0.16.9", recommended: [] }))).not.toBe("");
    expect(issues(packSchema, pack({ name: "ruff", version: "0.16.9", recommended: ["default", "default"] }))).toContain("listed once each");
  });

  it("keeps every packs/*/conformance.json valid", () => {
    for (const id of readdirSync(PACKS)) {
      const path = join(PACKS, id, "conformance.json");
      if (!existsSync(path)) continue;
      const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
      if (z.looseObject({ format: z.string() }).safeParse(parsed).success) continue;
      expect(issues(packSchema, parsed), path).toBe("");
    }
  });
});

describe("upstream/<tool>/source.json", () => {
  const source = (): UpstreamSource => ({
    tool: "ruff",
    version: "0.16.9",
    repository: "https://github.com/astral-sh/ruff",
    commit: "0be08a206f9c3180afd3e93bcc792ed5cb1f4db1",
    tag: "0.16.9",
    configurations: ["default"],
    extensions: [".py", ".pyi"],
    keywords: ["False", "None", "assert"],
    commentVocabulary: ["noqa", "type"],
    testWords: ["AssertionError", "assert", "foo"],
    configurationWords: [],
    sharedNames: ["Expr", "checker"],
    rules: {
      B011: {
        implementation: ["crates/ruff_linter/src/rules/flake8_bugbear/rules/assert_false.rs"],
        tests: ["crates/ruff_linter/resources/test/fixtures/flake8_bugbear/B011.py"],
        deprecated: null,
        enabledBy: null,
        names: { implementation: ["AssertionError", "assert_false"], shared: [] },
        regexes: [],
        unscored: [],
      },
      F841: {
        implementation: ["crates/ruff_linter/src/rules/pyflakes/rules/unused_variable.rs"],
        tests: ["crates/ruff_linter/resources/test/fixtures/pyflakes/F841_0.py"],
        deprecated: null,
        enabledBy: { configuration: "default", options: [{ "dummy-variable-rgx": "^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$" }] },
        names: { implementation: ["unused"], shared: ["Binding"] },
        regexes: [{ source: "^(_+|(_+[a-zA-Z0-9_]*[a-zA-Z0-9]+?))$", flags: "u" }],
        unscored: [{ test: "crates/ruff_notebook/resources/test/fixtures/jupyter/unused_variable.ipynb", case: null, reason: "notebook" }],
      },
    },
  });
  const changed = (edit: (value: UpstreamSource) => void): UpstreamSource => {
    const value = source();
    edit(value);
    return value;
  };

  it("accepts a tool's facts", () => {
    expect(issues(upstreamSourceSchema, source())).toBe("");
  });

  it.each<[string, (value: UpstreamSource) => void, string]>([
    ["an unsorted set", value => void (value.testWords = ["foo", "assert"]), '"assert" is repeated or out of order'],
    ["a repeated set member", value => void (value.keywords = ["None", "None"]), '"None" is repeated or out of order'],
    ["an abbreviated commit", value => void (value.commit = "0be08a2"), "a full lowercase commit sha"],
    ["a repository that is not a URL", value => void (value.repository = "astral-sh/ruff"), "repository"],
    ["no configuration", value => void (value.configurations = []), "configurations"],
    ["a repeated configuration", value => void (value.configurations = ["default", "default"]), "listed once each"],
    ["an enabling configuration it does not list", value => void (value.rules.F841!.enabledBy = { configuration: "all", options: [] }), "enabledBy names all"],
    ["an empty deprecation", value => void (value.rules.B011!.deprecated = ""), "deprecated"],
    ["a regex JavaScript cannot compile", value => void (value.rules.F841!.regexes = [{ source: "(?P<x>a)", flags: "u" }]), "not a JavaScript regex"],
    ["an unknown regex flag", value => void (value.rules.F841!.regexes = [{ source: "a", flags: "x" }]), "flags"],
    ["no rules", value => void (value.rules = {}), "rules is empty"],
    ["rules out of order", value => void (value.rules = { F841: value.rules.F841!, B011: value.rules.B011! }), "rules: B011 is out of order"],
    ["no extensions", value => void (value.extensions = []), "extensions"],
    ["an unknown unscored reason", value => void (value.rules.F841!.unscored = [{ test: "a.py", case: null, reason: "flaky" as "settings" }]), "reason"],
    [
      "unscored tests out of order",
      value => void (value.rules.F841!.unscored = [{ test: "t.js", case: "valid[1]", reason: "settings" }, { test: "t.js", case: "valid[0]", reason: "unparsable" }]),
      "t.js valid[0] is repeated or out of order",
    ],
  ])("refuses %s", (_name, edit, message) => {
    expect(issues(upstreamSourceSchema, changed(edit))).toContain(message);
  });

  it("refuses an unknown field, at the top and in a rule", () => {
    expect(issues(upstreamSourceSchema, { ...source(), license: "MIT" })).toContain("license");
    expect(issues(upstreamSourceSchema, changed(value => Object.assign(value.rules.B011!, { libraryNames: [] })))).toContain("libraryNames");
  });

  it("loads a source.json, refusing a repeated key", () => {
    const text = JSON.stringify(source());
    expect(loadUpstreamSource(join(written("source.json", text), "source.json")).version).toBe("0.16.9");
    const repeated = text.replace('"tag":"0.16.9"', '"tag":"0.16.9","tag":"0.16.8"');
    expect(() => loadUpstreamSource(join(written("source.json", repeated), "source.json"))).toThrow(/tag/u);
  });
});

describe("scrub.json", () => {
  it("accepts sorted (rule, scrubbable sub-check) entries", () => {
    expect(
      issues(scrubSchema, [
        { rule: "python:ruff-b011", check: "provenance:recommended" },
        { rule: "python:ruff-sim222", check: "ports:evidence" },
        { rule: "python:ruff-sim222", check: "ports:span" },
      ]),
    ).toBe("");
  });

  it.each([
    ["a sub-check that is never scrubbed", [{ rule: "a", check: "ports:points" }], "check"],
    ["a pack-level sub-check", [{ rule: "a", check: "licences" }], "check"],
    ["a tool-level sub-check", [{ rule: "a", check: "provenance:pins" }], "check"],
    ["an unknown sub-check", [{ rule: "a", check: "ports:real-code" }], "check"],
    ["entries out of order", [{ rule: "b", check: "ports:span" }, { rule: "a", check: "ports:span" }], "repeated or out of order"],
    ["a repeated entry", [{ rule: "a", check: "ports:span" }, { rule: "a", check: "ports:span" }], "repeated or out of order"],
    ["a reason", [{ rule: "a", check: "ports:span", reason: "vibed" }], "reason"],
  ])("refuses %s", (_name, entries, message) => {
    expect(issues(scrubSchema, entries)).toContain(message);
  });

  it("loads nothing when the pack has none, and refuses a rule pack.json does not list or a repeated key", () => {
    const rules = new Set(["a"]);
    expect(loadScrub(written("other.json", "[]"), rules)).toEqual([]);
    expect(loadScrub(written(SCRUB_FILE, '[{"rule":"a","check":"ports:span"}]\n'), rules)).toEqual([{ rule: "a", check: "ports:span" }]);
    expect(() => loadScrub(written(SCRUB_FILE, '[{"rule":"b","check":"ports:span"}]'), rules)).toThrow("scrub.json: b not listed in pack.json");
    expect(() => loadScrub(written(SCRUB_FILE, '[{"rule":"a","rule":"a","check":"ports:span"}]'), rules)).toThrow(/rule/u);
  });
});

describe("the pack validation report", () => {
  const results = (outcomes: Partial<Record<RuleSubCheckId, [SubCheckOutcome, ...string[]]>>, otherwise: SubCheckOutcome = "ok") =>
    RULE_SUB_CHECK_IDS.map(subCheck => {
      const [outcome, ...details] = outcomes[subCheck] ?? [otherwise];
      const filled = details.length > 0 || outcome === "ok" ? details : [`${outcome} because`];
      return { subCheck, outcome, details: filled };
    });

  const rule = (id: string, status: RuleReport["status"], stoppedAt: number | null, outcomes: Parameters<typeof results>[0]): RuleReport => ({
    id,
    document: `rules/${id}.json`,
    sha256: "a".repeat(64),
    bytes: 512,
    port: { id, tool: "ruff", rule: "B011" },
    status,
    stoppedAt,
    results: results(outcomes),
    generality: null,
    size: null,
    ports: null,
  });

  const zeros = <K extends string>(keys: readonly K[]): Record<K, number> => Object.fromEntries(keys.map(key => [key, 0])) as Record<K, number>;

  const report = (): ValidationReport => {
    const rules = [
      rule("python:a", "ok", null, {}),
      rule("python:b", "scrubbed", 4, { "ports:span": ["scrubbed", "3 ends differ"], "provenance:recommended": ["scrubbed", "not on in default"] }),
      rule("python:c", "failed", 7, { "provenance:upstream-rule": ["failed", "no B999 in source.json"], "size:written-out-repetition": ["n/a", "under the flag"] }),
    ];
    return {
      format: VALIDATION_REPORT_FORMAT,
      pack: "python",
      engine: { commit: "495ef0a5bebc1b74ee9b521322ee145923ed4701", dirty: false },
      tools: [
        { name: "ruff", version: "0.16.9", recommended: ["default"], source: { repository: "https://github.com/astral-sh/ruff", commit: "0be08a206f9c3180afd3e93bcc792ed5cb1f4db1", tag: "0.16.9" } },
        { name: "bandit", version: "1.8.6", recommended: null, source: null },
      ],
      notJudged: [{ check: "provenance", detail: "no source facts recorded for bandit" }],
      packFailures: [],
      scrub: {
        fidelity: [{ rule: "python:b", check: "ports:span" }],
        "upstream-policy": [{ rule: "python:b", check: "provenance:recommended" }],
      },
      summary: {
        rules: 3,
        ok: 1,
        failed: 1,
        scrubbed: 1,
        notSupportedYet: 0,
        notJudged: 0,
        stoppedAt: { ...zeros(PACK_CHECKS.map(check => check.name)), ports: 1, provenance: 1 },
        failedBy: { ...zeros(RULE_SUB_CHECK_IDS), "provenance:upstream-rule": 1 },
        scrubbedBy: { ...zeros(RULE_SUB_CHECK_IDS), "ports:span": 1, "provenance:recommended": 1 },
      },
      rules,
      pass: false,
    };
  };
  const changed = (edit: (value: ValidationReport) => void): ValidationReport => {
    const value = report();
    edit(value);
    return value;
  };

  it("accepts a consistent report", () => {
    expect(issues(validationReportSchema, report())).toBe("");
  });

  it("knows every outcome", () => {
    expect(SUB_CHECK_OUTCOMES).toEqual(["ok", "failed", "scrubbed", "n/a", "not-judged", "not-supported-yet"]);
  });

  it.each<[string, (value: ValidationReport) => void, string]>([
    ["a date", value => Object.assign(value, { date: "2026-10-08" }), "date"],
    ["a missing sub-check result", value => void value.rules[0]!.results.pop(), "results are not one per rule-level sub-check"],
    ["a status its results contradict", value => void (value.rules[1]!.status = "ok"), "python:b: status ok, but its results make it scrubbed"],
    ["a wrong stoppedAt", value => void (value.rules[2]!.stoppedAt = 9), "python:c: stoppedAt 9, but its results stop at 7"],
    ["a failure without its detail", value => void (value.rules[2]!.results[10]!.details = []), "provenance:upstream-rule: failed takes at least one detail"],
    ["an ok with a detail", value => void (value.rules[0]!.results[0]!.details = ["fine"]), "rules: ok takes no details"],
    ["a not-judged without its reason", value => void (value.rules[0]!.results[12] = { subCheck: "provenance:recommended", outcome: "not-judged", details: [] }), "provenance:recommended: not-judged takes exactly one detail"],
    ["a status count the rules contradict", value => void (value.summary.ok = 2), "summary.ok is 2; 1 rules are ok"],
    ["a not-judged count the rules contradict", value => void (value.summary.notJudged = 1), "summary.notJudged is 1"],
    ["a funnel the rules contradict", value => void (value.summary.stoppedAt.size = 1), "summary.stoppedAt.size is 1"],
    ["a failedBy the rules contradict", value => void (value.summary.failedBy["ports:points"] = 1), "summary.failedBy.ports:points"],
    ["a pass despite a failed rule", value => void (value.pass = true), "pass is true, but the failures make it false"],
    ["a scrub entry of a sub-check never scrubbed", value => void (value.scrub.fidelity = [{ rule: "python:b", check: "ports:points" as "ports:span" }]), "check"],
    ["an unknown check not judged", value => void (value.notJudged = [{ check: "real-code" as "size", detail: "x" }]), "check"],
    ["a funnel missing a check", value => void delete (value.summary.stoppedAt as Partial<Record<string, number>>).size, "size"],
  ])("refuses %s", (_name, edit, message) => {
    expect(issues(validationReportSchema, changed(edit))).toContain(message);
  });

  it("fails a report with a pack failure, whatever its rules", () => {
    const value = changed(report => {
      report.rules = [report.rules[0]!];
      report.summary = { ...report.summary, rules: 1, failed: 0, scrubbed: 0, stoppedAt: zeros(PACK_CHECKS.map(check => check.name)), failedBy: zeros(RULE_SUB_CHECK_IDS), scrubbedBy: zeros(RULE_SUB_CHECK_IDS) };
      report.scrub = { fidelity: [], "upstream-policy": [] };
      report.packFailures = [{ subCheck: "provenance:pins", tool: "ruff", detail: "source.json records 0.16.8; conformance.json pins 0.16.9" }];
      report.pass = false;
    });
    expect(issues(validationReportSchema, value)).toBe("");
    expect(issues(validationReportSchema, { ...value, pass: true })).toContain("pass is true");
  });
});
