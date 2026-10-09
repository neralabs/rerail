/**
 * The pack validation contract (docs/pack-format.md): every packs/<id>/ directory through checks 1 to 9, one test per
 * check and one per rule. A check test fails on a failure scrub.json does not cover, and is skipped, naming why, when
 * the check is not judged for some of the pack's rules (no source facts recorded for a tool, or another harness scores
 * the ports). A rule test fails on any uncovered failure, passes a rule whose every failure is on scrub.json (its title
 * lists the entries), and skips a rule core cannot read yet. Check 4 scores ported rules on the tools' fixtures, and on
 * the pinned corpora that are checked out (RERAIL_PACK_CORPORA=1: every corpus, a missing checkout failing; 0: none).
 *
 * The validator's own tests run it over the fixture packs in test/fixtures/packs/: `valid` passes, each negative
 * breaks one sub-check of a copy of it, and `vue`, for a language core plans, passes with its rule not supported yet.
 */
import { cpSync, existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, test } from "vitest";
import { z } from "zod";
import { PACK_CHECKS, validatePack, type PackCheckName, type PackValidation, type ValidatePackOptions } from "../conformance/pack-validation.js";
import { renderValidationReport, validationReport, validationReportSchema } from "../conformance/validation-report.js";
import { compareDeterministicStrings } from "../src/stable-json.js";

const PACKS = fileURLToPath(new URL("../packs/", import.meta.url));
const FIXTURES = fileURLToPath(new URL("./fixtures/packs/", import.meta.url));
const TIMEOUT = 600_000;
const ENGINE = { commit: "0".repeat(40), dirty: false };

const CORPORA: ValidatePackOptions = process.env.RERAIL_PACK_CORPORA === "1" ? { corpora: true } : process.env.RERAIL_PACK_CORPORA === "0" ? { corpora: false } : {};

const packs = existsSync(PACKS)
  ? readdirSync(PACKS, { withFileTypes: true })
      .filter(entry => entry.isDirectory())
      .map(entry => entry.name)
      .sort(compareDeterministicStrings)
  : [];

/** A pack's JSON file read when tests are collected, or undefined when it does not parse; the checks report why. */
function collected<T>(path: string, schema: z.ZodType<T>): T | undefined {
  try {
    const parsed = schema.safeParse(JSON.parse(readFileSync(path, "utf8")));
    return parsed.success ? parsed.data : undefined;
  } catch {
    return undefined;
  }
}

/** The rules pack.json lists, each with the sub-checks scrub.json lists for it, for the tests' titles. */
function listedRules(id: string): { id: string; title: string }[] {
  const rules = collected(join(PACKS, id, "pack.json"), z.object({ rules: z.array(z.object({ id: z.string() })) }))?.rules ?? [];
  const scrub = collected(join(PACKS, id, "scrub.json"), z.array(z.object({ rule: z.string(), check: z.string() }))) ?? [];
  return rules.map(rule => {
    const entries = scrub.filter(entry => entry.rule === rule.id).map(entry => entry.check);
    return { id: rule.id, title: entries.length === 0 ? rule.id : `${rule.id} (scrubbed: ${entries.join(", ")})` };
  });
}

describe.each(packs)("pack %s", id => {
  let validation: Promise<PackValidation> | undefined;
  const validated = (): Promise<PackValidation> => (validation ??= validatePack(join(PACKS, id), CORPORA));

  test.for(PACK_CHECKS)("check $number: $name", { timeout: TIMEOUT }, async ({ name }, { skip }) => {
    const result = await validated();
    expect(result.checks[name]).toEqual([]);
    const reasons = result.notJudged[name];
    if (reasons !== undefined) skip(`not judged for some rules: ${reasons.join("; ")}`);
  });

  test.for(listedRules(id))("rule $title", { timeout: TIMEOUT }, async ({ id: ruleId }, { skip, annotate }) => {
    const rule = (await validated()).rules.find(entry => entry.id === ruleId);
    expect(rule, `${ruleId} is not in the validation`).toBeDefined();
    const results = rule!.results;
    if (rule!.status === "not-supported-yet") skip(`not supported yet: ${results.find(result => result.outcome === "not-supported-yet")!.details[0]!}`);
    expect(results.filter(result => result.outcome === "failed").flatMap(result => result.details.map(detail => `${result.subCheck}: ${detail}`))).toEqual([]);
    const scrubbed = results.filter(result => result.outcome === "scrubbed");
    if (scrubbed.length > 0) await annotate(`scrubbed: ${scrubbed.map(result => `${result.subCheck} (${result.details[0]!})`).join("; ")}`);
    const notJudged = results.filter(result => result.outcome === "not-judged");
    if (notJudged.length > 0) await annotate(`not judged: ${[...new Set(notJudged.map(result => `${result.subCheck.split(":")[0]!}: ${result.details[0]!}`))].join("; ")}`);
  });
});

describe("validatePack", () => {
  const temporary: string[] = [];
  afterEach(() => {
    for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
  });

  /** A copy of fixture pack `name`, in a directory of that name, with `change` applied to it. */
  function changed(name: string, change: (directory: string) => void): string {
    const parent = mkdtempSync(join(tmpdir(), "rerail-packs-"));
    temporary.push(parent);
    const directory = join(parent, name);
    cpSync(join(FIXTURES, name), directory, { recursive: true });
    change(directory);
    return directory;
  }

  const writeJson = (path: string, value: unknown): void => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
  const record = z.record(z.string(), z.unknown());
  const editJson = (path: string, edit: (value: Record<string, unknown>) => unknown): void => writeJson(path, edit(record.parse(JSON.parse(readFileSync(path, "utf8")))));
  const editText = (path: string, edit: (text: string) => string): void => writeFileSync(path, edit(readFileSync(path, "utf8")));
  const SOURCE = ["upstream", "examplelint", "source.json"];
  const RECORDING = ["upstream", "examplelint", "fixtures.expected.json"];
  const ASSERT_USED = ["rules", "assert-used.json"];

  /** pack.json with the `pass-statement` rule's listing replaced by `edit`'s result. */
  const editPassStatement = (directory: string, edit: (rule: Record<string, unknown>) => Record<string, unknown>): void =>
    editJson(join(directory, "pack.json"), pack => ({ ...pack, rules: z.array(record).parse(pack.rules).map(rule => (rule.id === "valid:pass-statement" ? edit(rule) : rule)) }));
  /** source.json with examplelint's EX101 facts replaced by `edit`'s result. */
  const editFacts = (directory: string, edit: (facts: Record<string, unknown>) => Record<string, unknown>): void =>
    editJson(join(directory, ...SOURCE), source => ({ ...source, rules: { EX101: edit(record.parse(record.parse(source.rules).EX101)) } }));
  /** assert-used's document with its only clause's rule replaced by `edit`'s result. */
  const editAssertRule = (directory: string, edit: (rule: Record<string, unknown>) => Record<string, unknown>, extra: Record<string, unknown> = {}): void =>
    editJson(join(directory, ...ASSERT_USED), document => {
      const [clause] = z.array(record).parse(document.clauses);
      const forbid = record.parse(clause!.forbid);
      const ast = record.parse(forbid.ast);
      return { ...document, ...extra, clauses: [{ ...clause, forbid: { ...forbid, ast: { ...ast, rule: edit(record.parse(ast.rule)) } } }] };
    });
  const editFindings = (directory: string, edit: (findings: Record<string, unknown>[]) => Record<string, unknown>[]): void =>
    editJson(join(directory, ...RECORDING), recording => ({ ...recording, findings: edit(z.array(record).parse(recording.findings)) }));
  const divergence = (path: string, line: number, kind: string) => ({ rule: "assert-used", source: "fixtures", path, line, kind, evidence: `a divergence the validator's own tests declare on ${path}:${line}` });
  /** A description long enough to put a document over check 9's 15,360-byte flag. */
  const LONG = { description: `Port of examplelint's EX101, padded over the size flag: ${"x".repeat(16_000)}` };

  const failing = (validation: PackValidation): Partial<Record<PackCheckName, string[]>> =>
    Object.fromEntries(Object.entries(validation.checks).filter(([, failures]) => failures.length > 0));
  const statuses = (validation: PackValidation) => validation.rules.map(({ id, status, stoppedAt }) => ({ id, status, stoppedAt }));

  it("passes a valid pack, every rule ok and every sub-check judged", async () => {
    const validation = await validatePack(join(FIXTURES, "valid"), { corpora: false });
    expect(failing(validation)).toEqual({});
    expect(validation.notJudged).toEqual({});
    expect(validation.pass).toBe(true);
    expect(statuses(validation)).toEqual([
      { id: "valid:assert-used", status: "ok", stoppedAt: null },
      { id: "valid:pass-statement", status: "ok", stoppedAt: null },
    ]);
    const outcomes = (id: string) => Object.fromEntries(validation.rules.find(rule => rule.id === id)!.results.map(result => [result.subCheck, result.outcome]));
    expect(Object.entries(outcomes("valid:assert-used")).filter(([, outcome]) => outcome !== "ok")).toEqual([["examples", "n/a"]]);
    expect(Object.entries(outcomes("valid:pass-statement")).filter(([, outcome]) => outcome === "ok").map(([subCheck]) => subCheck)).toEqual(["rules", "rules:deterministic", "proof", "examples"]);
    expect(validation.rules[0]!.ports).toMatchObject({ parts: ["line", "column", "endLine", "endColumn"], units: 2, expected: 3, matched: 3, evidence: 1 });
  });

  it("passes a pack for a planned language, its rules not supported yet and their later checks waiting", async () => {
    const validation = await validatePack(join(FIXTURES, "vue"), { corpora: false });
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
    const [rule] = validation.rules;
    expect({ id: rule!.id, status: rule!.status }).toEqual({ id: "vue:no-v-html", status: "not-supported-yet" });
    expect(rule!.results.find(result => result.subCheck === "examples")).toEqual({
      subCheck: "examples",
      outcome: "not-supported-yet",
      details: ["/clauses/0/forbid/ast/language: `vue` is not supported yet"],
    });
    expect(rule!.results.filter(result => result.outcome === "ok").map(result => result.subCheck)).toEqual(["rules", "rules:deterministic", "proof"]);
  });

  const cases: [string, (directory: string) => void, Partial<Record<PackCheckName, string[]>>][] = [
    // Check 1.
    [
      "a rule file pack.json does not list",
      directory => writeFileSync(join(directory, "rules", "extra.json"), readFileSync(join(directory, "rules", "pass-statement.json"))),
      { rules: ["rules/extra.json is not listed in pack.json"] },
    ],
    [
      "a missing document",
      directory => rmSync(join(directory, "rules", "pass-statement.json")),
      {
        rules: ["valid:pass-statement: rules/pass-statement.json does not exist", "rules:deterministic: valid:pass-statement: not run: rules/pass-statement.json does not exist"],
        examples: ["valid:pass-statement: not run: the document fails check 1"],
      },
    ],
    [
      "a document whose id differs from the listing",
      directory => editJson(join(directory, "rules", "pass-statement.json"), document => ({ ...document, id: "valid:pass" })),
      { rules: ["valid:pass-statement: rules/pass-statement.json has id valid:pass", 'valid:pass-statement: rules/pass-statement.json compiles to id "valid:pass"'] },
    ],
    [
      "a document that repeats a key",
      directory => {
        const path = join(directory, "rules", "pass-statement.json");
        writeFileSync(path, readFileSync(path, "utf8").replace('"mode": "steer",', '"mode": "log",\n  "mode": "steer",'));
      },
      {
        rules: [
          "valid:pass-statement: rules/pass-statement.json repeats a key: duplicate key `mode` at /",
          "valid:pass-statement: rules/pass-statement.json does not compile: duplicate key `mode` at / at line 6 column 8",
        ],
        examples: ["valid:pass-statement: not run: the document fails check 1"],
      },
    ],
    [
      "a clause that needs a model",
      directory =>
        editJson(join(directory, "rules", "pass-statement.json"), document => ({
          ...document,
          clauses: [
            ...z.array(z.unknown()).parse(document.clauses),
            { id: "judged", llm: { prompt: "Is this pass statement needed?" }, violation: { anchor: "witness.span", code: "PASS", message: "a needless pass statement" } },
          ],
        })),
      {
        rules: ["rules:deterministic: valid:pass-statement: rules/pass-statement.json clause 1 is an llm clause, which needs a model at evaluation time"],
        examples: ["caught", "passes"].map(
          side =>
            `valid:pass-statement: the ${side} example (src/a.py) fails in the engine: [{"clauseId":"judged","code":"missing-capability","message":"LLM capability is unavailable"}]`,
        ),
      },
    ],
    [
      "a port that does not apply to every file type its tool lints",
      directory => editJson(join(directory, ...ASSERT_USED), document => ({ ...document, scope: { include: ["**/*.py"], exclude: [] } })),
      { rules: ["rules:in-scope: valid:assert-used: rules/assert-used.json does not apply to src/example.pyi"] },
    ],
    [
      "a port that does not apply to a file type its tool lints outside its language's dialects",
      directory => {
        editJson(join(directory, ...SOURCE), source => ({ ...source, extensions: [".py", ".pyi", ".pyw"] }));
        editJson(join(directory, ...ASSERT_USED), document => ({ ...document, scope: { include: ["**/*.py", "**/*.pyi"], exclude: [] } }));
      },
      { rules: ["rules:in-scope: valid:assert-used: rules/assert-used.json does not apply to src/example.pyw"] },
    ],
    [
      "a port of a tool without source facts that does not apply to every file of its language",
      directory => {
        editJson(join(directory, "conformance.json"), pack => ({ ...pack, tools: [{ name: "examplelint", version: "1.0.0" }] }));
        editJson(join(directory, ...ASSERT_USED), document => ({ ...document, scope: { include: ["**/*.py"], exclude: [] } }));
      },
      { rules: ["rules:in-scope: valid:assert-used: rules/assert-used.json does not apply to src/example.pyi"] },
    ],
    // Checks 2 and 3.
    [
      "a rule with neither examples nor a port entry",
      directory => editPassStatement(directory, ({ examples: _examples, ...rule }) => rule),
      { proof: ["valid:pass-statement: no examples and no port entry in conformance.json"] },
    ],
    [
      "a caught example that does not fire",
      directory => editPassStatement(directory, rule => ({ ...rule, examples: { caught: { path: "src/a.py", text: "x = 1\n" }, passes: { path: "src/a.py", text: "x = 2\n" } } })),
      { examples: ["valid:pass-statement: the caught example (src/a.py) produces no PASS violation"] },
    ],
    [
      "a passes example that fires",
      directory => editPassStatement(directory, rule => ({ ...rule, examples: { caught: { path: "src/a.py", text: "pass\n" }, passes: { path: "src/a.py", text: "pass\n" } } })),
      { examples: ["valid:pass-statement: the passes example (src/a.py) produces PASS"] },
    ],
    // Check 4.
    [
      "a ported rule's violation the recording lacks, outside divergences.json",
      directory => writeFileSync(join(directory, "divergences.json"), "[]\n"),
      { ports: ['ports:points: assert-used (fixtures): 1 unexplained FP, first {"path":"asserts.py","line":5,"column":5}'] },
    ],
    [
      "a stale divergence",
      directory => {
        const path = join(directory, "divergences.json");
        writeJson(path, [...z.array(z.unknown()).parse(JSON.parse(readFileSync(path, "utf8"))), divergence("asserts.py", 3, "upstream-wrong")]);
      },
      { ports: ["ports:points: assert-used: stale divergence asserts.py:3 (upstream-wrong) explains nothing"] },
    ],
    [
      "a finding Rerail reproduces with another end",
      directory => editFindings(directory, findings => findings.map(finding => (finding.path === "asserts.py" && finding.line === 1 ? { ...finding, endColumn: 7 } : finding))),
      {
        ports: [
          'ports:span: assert-used (fixtures): 1 finding(s) agree at line+column but differ at endLine, endColumn, first upstream {"path":"asserts.py","line":1,"column":1,"endLine":1,"endColumn":7}, Rerail {"path":"asserts.py","line":1,"column":1,"endLine":1,"endColumn":9}',
        ],
      },
    ],
    [
      "an upstream finding Rerail misses that only a rerail-unknown divergence explains",
      directory => {
        editFindings(directory, findings => [...findings, { rule: "EX101", path: "asserts.py", line: 2, column: 1, endLine: 2, endColumn: 6 }]);
        const path = join(directory, "divergences.json");
        writeJson(path, [...z.array(z.unknown()).parse(JSON.parse(readFileSync(path, "utf8"))), divergence("asserts.py", 2, "rerail-unknown")]);
      },
      {
        ports: [
          'ports:unknown-misses: assert-used (fixtures): 1 upstream finding(s) missed and explained only by a rerail-unknown divergence, first {"path":"asserts.py","line":2,"column":1}',
        ],
      },
    ],
    [
      "a port no fixture unit agrees on completely",
      directory => {
        editFindings(directory, () => []);
        writeJson(join(directory, "divergences.json"), [
          divergence("asserts.py", 1, "upstream-wrong"),
          divergence("asserts.py", 3, "upstream-wrong"),
          divergence("asserts.py", 5, "upstream-wrong"),
          divergence("clean.py", 2, "upstream-wrong"),
        ]);
      },
      { ports: ["ports:evidence: assert-used (fixtures): no upstream finding is reproduced in a unit where the port agrees completely (0 of 2 unit(s) agree; upstream reports 0 finding(s))"] },
    ],
    [
      "a declared capability with neither a ported rule nor an exemption",
      directory => editJson(join(directory, "conformance.json"), pack => ({ ...pack, capabilities: ["syntax", "scope"] })),
      { ports: ["capability scope: no ported rule exercises it and it has no exemption"] },
    ],
    // Checks 5 and 6.
    [
      "dependencies in package.json",
      directory => editJson(join(directory, "package.json"), manifest => ({ ...manifest, dependencies: { "left-pad": "1.3.0" } })),
      { package: ["package.json: has dependencies (left-pad); a pack has none"] },
    ],
    [
      "scrub.json in package.json files",
      directory => editJson(join(directory, "package.json"), manifest => ({ ...manifest, files: [...z.array(z.string()).parse(manifest.files), "scrub.json"] })),
      { package: ["package.json: files entry scrub.json publishes scrub.json, which is validation material"] },
    ],
    [
      "a licence that disagrees with licenses/",
      directory => editJson(join(directory, "package.json"), manifest => ({ ...manifest, license: "Apache-2.0" })),
      { package: ['package.json: license "Apache-2.0" names Apache-2.0; the pack carries Apache-2.0, MIT (Apache-2.0 and what licenses/*/UPSTREAM.md records)'] },
    ],
    [
      "a copied file no UPSTREAM.md lists",
      directory => writeFileSync(join(directory, "upstream", "examplelint", "fixtures", "more.py"), "assert d\n"),
      { licences: ["upstream/examplelint/fixtures/more.py is copied but no licenses/*/UPSTREAM.md lists it"] },
    ],
    [
      "a source.json no UPSTREAM.md lists",
      directory => editText(join(directory, "licenses", "examplelint", "UPSTREAM.md"), text => text.replace(/^\| `upstream\/examplelint\/source\.json`.*\n/mu, "")),
      { licences: ["upstream/examplelint/source.json is extracted from upstream but no licenses/*/UPSTREAM.md lists it"] },
    ],
    // Check 7.
    [
      "source facts of another version than conformance.json pins",
      directory => editJson(join(directory, ...SOURCE), source => ({ ...source, version: "2.0.0" })),
      {
        provenance: [
          "provenance:pins: examplelint: upstream/examplelint/source.json records version 2.0.0; conformance.json pins 1.0.0",
          "provenance:pins: examplelint: upstream/examplelint/fixtures.expected.json was recorded with version 1.0.0; source.json records 2.0.0",
        ],
      },
    ],
    [
      "a tool that declares source facts without a source.json",
      directory => rmSync(join(directory, ...SOURCE)),
      {
        licences: ["licenses/examplelint/UPSTREAM.md lists upstream/examplelint/source.json, which does not exist"],
        ports: ["ports:coverage: valid:assert-used: not run: examplelint has no valid source.json (provenance:pins)"],
        provenance: [
          "provenance:pins: examplelint: upstream/examplelint/source.json is missing; the tool declares recommended configurations",
          ...["upstream-rule", "stable", "recommended"].map(name => `provenance:${name}: valid:assert-used: not run: examplelint has no valid source.json (provenance:pins)`),
        ],
        generality: ["names", "regexes"].map(name => `generality:${name}: valid:assert-used: not run: examplelint has no valid source.json (provenance:pins)`),
        size: ["combinatorial-expansion", "written-out-repetition"].map(name => `size:${name}: valid:assert-used: not run: examplelint has no valid source.json (provenance:pins)`),
      },
    ],
    [
      "upstream tests check 4 does not score",
      directory =>
        editFacts(directory, facts => ({
          ...facts,
          unscored: [
            { test: "tests/asserts.ipynb", case: null, reason: "notebook" },
            { test: "tests/settings.py", case: null, reason: "settings" },
            { test: "tests/x.py", case: "invalid[0]", reason: "unparsable" },
            { test: "tests/x.py", case: "invalid[1]", reason: "unparsable" },
            { test: "tests/x.py", case: "invalid[2]", reason: "unparsable" },
            { test: "tests/x.py", case: "invalid[3]", reason: "unparsable" },
          ],
        })),
      {
        ports: [
          "ports:coverage: valid:assert-used: 1 upstream test(s) not among the fixtures check 4 scores, a Jupyter notebook, which no dialect reads: tests/asserts.ipynb",
          "ports:coverage: valid:assert-used: 1 upstream test(s) not among the fixtures check 4 scores, upstream's tests lint it only with settings other than the approved configuration's: tests/settings.py",
          "ports:coverage: valid:assert-used: 4 upstream test(s) not among the fixtures check 4 scores, the pack's configuration cannot parse the case's code: tests/x.py invalid[0]; tests/x.py invalid[1]; tests/x.py invalid[2]; and 1 more",
        ],
      },
    ],
    [
      "an upstream rule without tests",
      directory => editFacts(directory, facts => ({ ...facts, tests: [] })),
      { provenance: ["provenance:upstream-rule: valid:assert-used: implementation: examplelint/rules/ex101.py; 0 upstream test file(s)"] },
    ],
    [
      "a deprecated upstream rule",
      directory => editFacts(directory, facts => ({ ...facts, deprecated: "replaced by EX102" })),
      { provenance: ["provenance:stable: valid:assert-used: upstream deprecates the rule: replaced by EX102"] },
    ],
    [
      "an upstream rule no approved configuration turns on",
      directory => editFacts(directory, facts => ({ ...facts, enabledBy: null })),
      { provenance: ["provenance:recommended: valid:assert-used: not on in any approved upstream configuration"] },
    ],
    // Check 8.
    [
      "a name only upstream's tests spell",
      directory => {
        editAssertRule(directory, rule => ({ ...rule, not: { regex: "^assert fixture_only_name$" } }));
        editJson(join(directory, ...SOURCE), source => ({ ...source, testWords: [...z.array(z.string()).parse(source.testWords), "fixture_only_name"].sort(compareDeterministicStrings) }));
      },
      { generality: ["generality:names: valid:assert-used: matches 1 name(s) that occur in upstream's tests but not in upstream's code: fixture_only_name"] },
    ],
    [
      "a regex the gate cannot run",
      directory => editAssertRule(directory, rule => ({ ...rule, not: { regex: "(?x) ^ f o o $" } })),
      { generality: ["generality:regexes: valid:assert-used: cannot judge 1 regex(es) written in syntax the gate cannot run, e.g. /clauses/0/forbid/ast/rule/not/regex: (?x) ^ f o o $"] },
    ],
    // Check 9.
    [
      "a large document that writes one node kind out along a nesting path",
      directory => {
        let chain: Record<string, unknown> = { kind: "assert_statement" };
        for (let level = 0; level < 3; level += 1) chain = { kind: "parenthesized_expression", has: chain };
        editAssertRule(directory, rule => ({ any: [rule, chain] }), LONG);
      },
      {
        size: [
          "size:written-out-repetition: valid:assert-used: 17063 bytes (16565 canonical) is over the 15360-byte flag, and one node kind occurs 3 times along a single nesting path (limit 3)",
        ],
      },
    ],
    [
      "a large document that writes a product out cell by cell",
      directory => {
        const cells = ["zq_window", "zq_self", "zq_top", "zq_parent"].flatMap(receiver =>
          ["zq_set_timeout", "zq_set_interval", "zq_post_message"].map(method => ({ kind: "call", regex: `^${receiver}\\.${method}\\b`, inside: { kind: "module", stopBy: "end" } })),
        );
        editAssertRule(directory, rule => ({ any: [rule, ...cells] }), LONG);
      },
      {
        size: [
          "size:combinatorial-expansion: valid:assert-used: 19551 bytes (17592 canonical) is over the 15360-byte flag, and 77.4% of the clauses is structure written out over and over (limit 33.3%)",
        ],
      },
    ],
  ];

  it.each(cases)("fails %s", async (_name, change, expected) => {
    const validation = await validatePack(changed("valid", change), { corpora: false });
    expect(failing(validation)).toEqual(expected);
    expect(validation.pass).toBe(false);
  });

  it("marks the rule a failure belongs to, and the check it stopped at", async () => {
    const validation = await validatePack(
      changed("valid", directory => writeFileSync(join(directory, "divergences.json"), "[]\n")),
      { corpora: false },
    );
    expect(statuses(validation)).toEqual([
      { id: "valid:assert-used", status: "failed", stoppedAt: 4 },
      { id: "valid:pass-statement", status: "ok", stoppedAt: null },
    ]);
  });

  it("probes a port with the file types its tool lints, not every dialect of its language", async () => {
    const validation = await validatePack(
      changed("valid", directory => {
        editJson(join(directory, ...SOURCE), source => ({ ...source, extensions: [".py"] }));
        editJson(join(directory, ...ASSERT_USED), document => ({ ...document, scope: { include: ["**/*.py"], exclude: [] } }));
      }),
      { corpora: false },
    );
    expect(failing(validation)).toEqual({});
    expect(validation.pass).toBe(true);
  });

  it("does not judge checks 7 to 9 and check 4's coverage for a tool without source facts, and never passes them", async () => {
    const validation = await validatePack(
      changed("valid", directory => editJson(join(directory, "conformance.json"), pack => ({ ...pack, tools: [{ name: "examplelint", version: "1.0.0" }] }))),
      { corpora: false },
    );
    expect(failing(validation)).toEqual({});
    const reason = ["no source facts recorded for examplelint"];
    expect(validation.notJudged).toEqual({ ports: reason, provenance: reason, generality: reason, size: reason });
    const rule = validation.rules.find(entry => entry.id === "valid:assert-used")!;
    expect(rule.status).toBe("ok");
    expect(rule.results.filter(result => result.outcome === "not-judged").map(result => result.subCheck)).toEqual([
      "ports:coverage",
      "provenance:upstream-rule",
      "provenance:stable",
      "provenance:recommended",
      "generality:names",
      "generality:regexes",
      "size:combinatorial-expansion",
      "size:written-out-repetition",
    ]);
    const report = validationReport(validation, ENGINE);
    expect(report.summary.notJudged).toBe(1);
    expect(report.tools).toEqual([{ name: "examplelint", version: "1.0.0", recommended: null, source: null }]);
  });

  describe("scrub.json", () => {
    const notRecommended = (directory: string): void => editFacts(directory, facts => ({ ...facts, enabledBy: null }));

    it("scrubs a covered failure: the rule passes as scrubbed and the report groups the entry", async () => {
      const validation = await validatePack(
        changed("valid", directory => {
          notRecommended(directory);
          writeJson(join(directory, "scrub.json"), [{ rule: "valid:assert-used", check: "provenance:recommended" }]);
        }),
        { corpora: false },
      );
      expect(failing(validation)).toEqual({});
      expect(validation.pass).toBe(true);
      expect(statuses(validation)[0]).toEqual({ id: "valid:assert-used", status: "scrubbed", stoppedAt: 7 });
      const report = validationReport(validation, ENGINE);
      expect(report.scrub).toEqual({ fidelity: [], "upstream-policy": [{ rule: "valid:assert-used", check: "provenance:recommended" }] });
      expect(report.summary).toMatchObject({ scrubbed: 1, failed: 0, scrubbedBy: { "provenance:recommended": 1 } });
    });

    it("fails an uncovered failure beside a covered one", async () => {
      const validation = await validatePack(
        changed("valid", directory => {
          editFacts(directory, facts => ({ ...facts, enabledBy: null, deprecated: "replaced by EX102" }));
          writeJson(join(directory, "scrub.json"), [{ rule: "valid:assert-used", check: "provenance:recommended" }]);
        }),
        { corpora: false },
      );
      expect(failing(validation)).toEqual({ provenance: ["provenance:stable: valid:assert-used: upstream deprecates the rule: replaced by EX102"] });
      expect(statuses(validation)[0]).toEqual({ id: "valid:assert-used", status: "failed", stoppedAt: 7 });
    });

    it("fails a stale entry, whose rule passes the sub-check", async () => {
      const validation = await validatePack(
        changed("valid", directory => writeJson(join(directory, "scrub.json"), [{ rule: "valid:assert-used", check: "ports:span" }])),
        { corpora: false },
      );
      expect(failing(validation)).toEqual({ ports: ["ports:span: valid:assert-used: passes; remove the scrub.json entry"] });
    });

    it("fails an entry for a sub-check that does not apply to its rule", async () => {
      const validation = await validatePack(
        changed("valid", directory => writeJson(join(directory, "scrub.json"), [{ rule: "valid:pass-statement", check: "ports:span" }])),
        { corpora: false },
      );
      expect(failing(validation)).toEqual({ ports: ["ports:span: valid:pass-statement: is n/a: the rule has no port entry; its examples prove it; remove the scrub.json entry"] });
    });

    it("fails a list that names a rule pack.json does not list, or a sub-check it may not cover", async () => {
      const unknown = await validatePack(changed("valid", directory => writeJson(join(directory, "scrub.json"), [{ rule: "valid:nope", check: "ports:span" }])), { corpora: false });
      expect(failing(unknown)).toEqual({ rules: ["scrub.json: valid:nope not listed in pack.json"] });
      const never = await validatePack(changed("valid", directory => writeJson(join(directory, "scrub.json"), [{ rule: "valid:assert-used", check: "ports:points" }])), { corpora: false });
      expect(never.checks.rules).toHaveLength(1);
      expect(never.checks.rules[0]).toMatch(/^scrub\.json: /u);
      expect(never.pass).toBe(false);
    });
  });

  it("writes the same report twice, and the report agrees with its own schema", async () => {
    const directory = changed("valid", directory => {
      editFacts(directory, facts => ({ ...facts, enabledBy: null }));
      writeJson(join(directory, "scrub.json"), [{ rule: "valid:assert-used", check: "provenance:recommended" }]);
    });
    const [first, second] = [await validatePack(directory, { corpora: false }), await validatePack(directory, { corpora: false })].map(validation => validationReport(validation, ENGINE));
    expect(JSON.stringify(second)).toBe(JSON.stringify(first));
    expect(renderValidationReport(second!)).toBe(renderValidationReport(first!));
    expect(validationReportSchema.safeParse(first).success).toBe(true);
    expect(renderValidationReport(first!)).toContain("### Upstream-policy exclusions\n\n- `valid:assert-used`: provenance:recommended");
  });
});
