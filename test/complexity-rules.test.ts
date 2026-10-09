/**
 * Parity of the complexity pack (packs/complexity/) with lizard 1.24.0. The corpus under
 * test/fixtures/complexity/<language>/ is scored by lizard once, by hand (scripts/record-lizard-complexity.mjs), into
 * lizard-1.24.0.tsv; lizard is never a test or runtime dependency. Each rule must report exactly the in-scope functions
 * lizard scores above 6, with lizard's number in the message, apart from the deviations listed here; a measuring variant
 * of each rule (same counting, no threshold) must give every function lizard's number, so functions at the limit and
 * below are checked too. Test files are out of scope even where lizard scores them over the limit.
 */
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { languageFor } from "../src/code-rule-enforcer.js";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../src/native.js";
import { loadBundledRules } from "../src/bundled-rule-documents.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../code-rule-dsl/", import.meta.url));
const CORPUS = fileURLToPath(new URL("./fixtures/complexity/", import.meta.url));
const capabilities = readFileSync(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;

const LIMIT = 6;
const message = (ccn: number) => `Cyclomatic complexity ${ccn} (limit ${LIMIT}). Split this function into smaller ones or simplify its branching.`;
const RULES = {
  python: "complexity:cyclomatic-python",
  typescript: "complexity:cyclomatic-typescript",
  javascript: "complexity:cyclomatic-javascript",
} as const;
type CorpusLanguage = keyof typeof RULES;
const LANGUAGES = Object.keys(RULES) as CorpusLanguage[];

/**
 * Where Rerail's number differs from lizard's, on purpose: lizard 1.24.0 tokenizer and reader defects, each found on
 * real code. `lizard: null` is a function lizard does not list; `rerail: null` is one lizard lists that is not a
 * function.
 * - `??` and `??=` count 0: lizard reads each as two `?` and counts 2.
 * - A `?` inside a regex literal, and `?.` right after `)` or `]`, count 0: lizard reads each as a ternary.
 * - An arrow function whose expression body starts on a later line is scored on its own: lizard adds its conditions to
 *   the enclosing function and lists a one-line arrow, scored 1, on the line after.
 * - A Python def whose return annotation spans several lines, or whose parameter list breaks after a default holding
 *   brackets, is scored whole: lizard ends it inside its signature and scores it 1 (fixed in crapkit #72).
 */
const DEVIATIONS: ReadonlyArray<{ path: string; line: number; lizard: number | null; rerail: number | null }> = [
  { path: "javascript/nullish.js", line: 4, lizard: 8, rerail: 4 },
  { path: "javascript/nullish.js", line: 17, lizard: 7, rerail: 5 },
  { path: "typescript/nullish.ts", line: 3, lizard: 8, rerail: 4 },
  { path: "typescript/nullish.ts", line: 16, lizard: 7, rerail: 5 },
  { path: "python/signatures.py", line: 1, lizard: 1, rerail: 7 },
  { path: "python/signatures.py", line: 35, lizard: 1, rerail: 8 },
  ...(["javascript/tokens.js", "typescript/tokens.ts"] as const).flatMap(path => [
    { path, line: 1, lizard: 2, rerail: 1 },
    { path, line: 5, lizard: 2, rerail: 1 },
    { path, line: 9, lizard: 2, rerail: 1 },
    { path, line: 13, lizard: 3, rerail: 1 },
    { path, line: 14, lizard: null, rerail: 3 },
    { path, line: 15, lizard: 1, rerail: null },
  ]),
];

const lizardRowSchema = z.object({ path: z.string().min(1), name: z.string().min(1), line: z.number().int().positive(), ccn: z.number().int().positive() });
const lizardRows = readFileSync(join(CORPUS, "lizard-1.24.0.tsv"), "utf8")
  .split("\n")
  .filter(line => line !== "" && !line.startsWith("#"))
  .slice(1)
  .map(line => {
    const [path, name, start, ccn] = line.split("\t");
    return lizardRowSchema.parse({ path, name, line: Number(start), ccn: Number(ccn) });
  });

const evaluationSchema = z.object({
  violations: z.array(z.object({ code: z.string(), path: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

const key = (path: string, line: number) => `${path}:${line}`;

/** Rerail's expected number for a function lizard scored: lizard's, or the deviation's (null: not a function). */
function rerailNumber(row: z.infer<typeof lizardRowSchema>): number | null {
  const deviation = DEVIATIONS.find(candidate => candidate.path === row.path && candidate.line === row.line);
  return deviation === undefined ? row.ccn : deviation.rerail;
}

/** Rerail's number for every function in `language`'s in-scope corpus: lizard's rows, corrected by the deviations. */
function expectedNumbers(language: CorpusLanguage, inScope: (path: string) => boolean): Record<string, number> {
  const expected: Record<string, number> = {};
  for (const row of lizardRows.filter(candidate => candidate.path.startsWith(`${language}/`) && inScope(candidate.path))) {
    const ccn = rerailNumber(row);
    if (ccn !== null) expected[key(row.path, row.line)] = ccn;
  }
  for (const deviation of DEVIATIONS.filter(candidate => candidate.lizard === null && candidate.path.startsWith(`${language}/`))) {
    expected[key(deviation.path, deviation.line)] = deviation.rerail!;
  }
  return expected;
}

function corpusFiles(language: CorpusLanguage): { path: string; source: string }[] {
  const root = join(CORPUS, language);
  return readdirSync(root, { recursive: true, withFileTypes: true })
    .filter(entry => entry.isFile())
    .map(entry => relative(CORPUS, join(entry.parentPath, entry.name)).split("\\").join("/"))
    .sort()
    .map(path => ({ path, source: readFileSync(join(CORPUS, path), "utf8") }));
}

/** Findings by `path:line` (1-based, as lizard prints it), each mapped to its message. */
function findings(compiled: NativeCompiledCodeRuleDsl, files: { path: string; source: string }[]): Record<string, string> {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: files.map(file => ({ language: languageFor(file.path) ?? "text", path: file.path, source: file.source })),
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  expect(result.violations.map(violation => violation.code).filter(code => code !== "CYCLOMATIC_COMPLEXITY")).toEqual([]);
  return Object.fromEntries(result.violations.map(violation => [key(violation.path, violation.span.startLine + 1), violation.message]));
}

const documents = loadBundledRules().filter(document => document.rule.id.startsWith("complexity:"));
function ruleJson(language: CorpusLanguage): string {
  const document = documents.find(candidate => candidate.rule.id === RULES[language]);
  if (document === undefined) throw new Error(`${RULES[language]} is not bundled`);
  return JSON.stringify(document.rule);
}

/** The shipped rule with its threshold removed: every function is a finding whose message carries its number. */
function measuringJson(language: CorpusLanguage): string {
  const shipped = ruleJson(language);
  const measuring = shipped.replace(/"min":6(?=[,}])/u, '"min":0,"max":1000000');
  expect(measuring, "the shipped count must read min 6").not.toBe(shipped);
  return measuring;
}

function compile(json: string): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  return runtime.compile(json, "json", capabilities);
}

describe("complexity rule pack", () => {
  it("records the deviations against lizard's own numbers", () => {
    for (const deviation of DEVIATIONS) {
      expect(lizardRows.find(row => row.path === deviation.path && row.line === deviation.line)?.ccn ?? null, key(deviation.path, deviation.line)).toBe(deviation.lizard);
    }
  });

  it.each(LANGUAGES)("%s: reports exactly the in-scope functions lizard scores over the limit, with lizard's number", language => {
    const compiled = compile(ruleJson(language));
    const expected = Object.fromEntries(
      Object.entries(expectedNumbers(language, path => compiled.inScope(path))).filter(([, ccn]) => ccn > LIMIT).map(([at, ccn]) => [at, message(ccn)]),
    );
    expect(Object.keys(expected).length, `${language} corpus has functions over the limit`).toBeGreaterThan(0);
    expect(findings(compiled, corpusFiles(language))).toEqual(expected);
  });

  it.each(LANGUAGES)("%s: measures every in-scope function as lizard does, apart from the listed deviations", language => {
    const compiled = compile(measuringJson(language));
    const expected = Object.fromEntries(Object.entries(expectedNumbers(language, path => compiled.inScope(path))).map(([at, ccn]) => [at, message(ccn)]));
    expect(findings(compiled, corpusFiles(language))).toEqual(expected);
  });

  it.each(LANGUAGES)("%s: leaves test files out even where lizard scores them over the limit", language => {
    const compiled = compile(ruleJson(language));
    const excluded = lizardRows.filter(row => row.path.startsWith(`${language}/`) && !compiled.inScope(row.path));
    expect(excluded.length, `${language} corpus has excluded test files`).toBeGreaterThan(0);
    for (const row of excluded) expect(row.ccn, key(row.path, row.line)).toBeGreaterThan(LIMIT);
    const files = corpusFiles(language);
    expect(findings(compiled, files.filter(file => excluded.some(row => row.path === file.path)))).toEqual({});
  });

  it.each([
    ["typescript", "typescript/__tests__/excluded.ts", ["src/widget.test.ts", "src/widget.spec.tsx", "src/__tests__/widget.mts"], "src/widget.cts"],
    ["javascript", "javascript/__tests__/excluded.js", ["src/widget.test.js", "src/widget.spec.jsx", "lib/widget.test.mjs"], "src/widget.cjs"],
  ] as const)("%s: excludes *.test.*, *.spec.* and __tests__ files, and scores the same source elsewhere", (language, fixture, testPaths, sourcePath) => {
    const compiled = compile(ruleJson(language));
    const source = readFileSync(join(CORPUS, fixture), "utf8");
    for (const path of testPaths) {
      expect(compiled.inScope(path), path).toBe(false);
      expect(findings(compiled, [{ path, source }]), path).toEqual({});
    }
    expect(compiled.inScope(sourcePath)).toBe(true);
    expect(findings(compiled, [{ path: sourcePath, source }])).toEqual({ [key(sourcePath, 3)]: message(8) });
  });

  it("scopes each rule to its own language's sources outside dependency and build output", () => {
    const scopes: Record<CorpusLanguage, { inside: string[]; outside: string[] }> = {
      python: { inside: ["app/service.py", "service.py"], outside: ["app/tests/service.py", "app/test_service.py", "app/service_test.py", "conftest.py", ".venv/lib/x.py", "app/service.ts"] },
      typescript: { inside: ["src/a.ts", "web/view.tsx", "lib/m.mts", "lib/c.cts"], outside: ["src/a.js", "node_modules/p/a.ts", "pkg/dist/a.ts", "src/generated/api.ts", "src/a.py"] },
      javascript: { inside: ["src/a.js", "web/view.jsx", "lib/m.mjs", "lib/c.cjs"], outside: ["src/a.ts", "node_modules/p/a.js", "pkg/dist/a.js", "src/a.min.js", "src/a.py"] },
    };
    for (const language of LANGUAGES) {
      const compiled = compile(ruleJson(language));
      for (const path of scopes[language].inside) expect(compiled.inScope(path), `${language} ${path}`).toBe(true);
      for (const path of scopes[language].outside) expect(compiled.inScope(path), `${language} ${path}`).toBe(false);
    }
  });
});
