/**
 * Parity of the complexity pack (packs/complexity/) with lizard, at the release its conformance.json pins. The corpus
 * under packs/complexity/upstream/lizard/fixtures/<language>/ is scored by lizard once, by hand (that directory's
 * generate.mjs), into lizard-<release>.tsv beside it; lizard is never a test or runtime dependency. Each rule must
 * report exactly the in-scope functions lizard scores above 6, with lizard's number in the message, apart from the
 * deviations listed here; a measuring variant of each rule (same counting, no threshold) must give every function
 * lizard's number, so functions at the limit and below are checked too. Test files are out of scope even where lizard
 * scores them over the limit. The conformance check judges the same rules on the findings alone
 * (fixtures.expected.json, with packs/complexity/divergences.json); this test checks every number.
 */
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { languageFor } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../src/engine/native/native.js";
import { loadPackRuleDocuments } from "../../../src/engine/catalog/rule-documents.js";
import { requirePackFixture } from "../../../scripts/s3/pack-fixtures.mjs";
import { repositoryPath } from "../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../native/src/code_rule_dsl/", import.meta.url));
const LIZARD = repositoryPath("packs", "complexity", "upstream", "lizard");
// Pack test data, not in git (the corpus authored, in S3; the TSV derived): a checkout without it fails here, saying to
// run `npm run fixtures:pull` and `npm run fixtures:derive`.
const CORPUS = requirePackFixture(repositoryPath("packs", "complexity", "upstream", "lizard", "fixtures"));
const capabilities = readFileSync(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;

const LIMIT = 6;
const RULES = {
  python: "complexity-cyclomatic-python",
  typescript: "complexity-cyclomatic-typescript",
  javascript: "complexity-cyclomatic-javascript",
  php: "complexity-cyclomatic-php",
  blade: "complexity-cyclomatic-blade",
  rust: "complexity-cyclomatic-rust",
  vue: "complexity-cyclomatic-vue",
} as const;
type CorpusLanguage = keyof typeof RULES;
const LANGUAGES = Object.keys(RULES) as CorpusLanguage[];

/** The steer for a function scored `ccn`: named as `name` (in backticks), or "This function" when it has none. */
function message(language: CorpusLanguage, name: string | undefined, ccn: number): string {
  const subject = name === undefined ? "This function" : `\`${name}\``;
  return `${subject} has cyclomatic complexity ${ccn}, over the ceiling of ${LIMIT}. Decompose it into smaller functions.`;
}

/** The number a steer carries; every message must be one `message` would write. */
function ccnOf(language: CorpusLanguage, text: string): number {
  const ccn = /has cyclomatic complexity (\d+),/u.exec(text)?.[1];
  const name = /^`([^`]+)` has/u.exec(text)?.[1];
  expect(text).toBe(message(language, name, Number(ccn)));
  return Number(ccn);
}

/**
 * Where Rerail's number differs from lizard's, on purpose: lizard 1.24.0 tokenizer and reader defects, each found on
 * real code or read from lizard's readers. `lizard: null` is a function lizard does not list; `rerail: null` is one
 * lizard lists that is not a function.
 * - `??` and `??=` count 0: lizard reads each as two `?` and counts 2.
 * - A `?` inside a regex literal, and `?.` right after `)` or `]`, count 0: lizard reads each as a ternary.
 * - An arrow function whose expression body starts on a later line is scored on its own: lizard adds its conditions to
 *   the enclosing function and lists a one-line arrow, scored 1, on the line after.
 * - A Python def whose return annotation spans several lines, or whose parameter list breaks after a default holding
 *   brackets, is scored whole: lizard ends it inside its signature and scores it 1 (fixed in crapkit #72).
 * - A default-only switch counts 0 beside a cased one. The recorded number is min(standard, -m) over the whole function,
 *   which there gives the default-only switch the 1 that -m adds for any switch; the rule counts each switch on its own.
 * - PHP: a nullable type's `?` counts 0, `and` and `or` count 1 each, a method named `match` counts 0, and a closure
 *   inside a function is scored on its own: lizard counts the `?`, skips `and`/`or`, counts the name, and adds the
 *   closure's decisions to the function around it.
 * - Blade: a function in a `@php` block or an echo is scored; lizard reads both as template text.
 * - Rust: `where` and `?Sized` count 0, a closure's empty `||` counts 0, and a macro's tokens are not read: lizard
 *   counts the first three and the `&&` and `if` in a macro. A byte literal is no lifetime: lizard reads `b'0'` as one
 *   and loses the `&&` after it. A trait's required method is no function: lizard runs it on into the next one, which
 *   it scores under the required method's line.
 * - Vue: a `<` directly before a letter is a comparison: lizard reads a tag up to the next `>` and loses what is between.
 */
const DEVIATIONS: ReadonlyArray<{ path: string; line: number; lizard: number | null; rerail: number | null }> = [
  { path: "javascript/nullish.js", line: 4, lizard: 8, rerail: 4 },
  { path: "javascript/nullish.js", line: 17, lizard: 7, rerail: 5 },
  { path: "typescript/nullish.ts", line: 3, lizard: 8, rerail: 4 },
  { path: "typescript/nullish.ts", line: 16, lizard: 7, rerail: 5 },
  { path: "python/signatures.py", line: 1, lizard: 1, rerail: 7 },
  { path: "python/signatures.py", line: 35, lizard: 1, rerail: 8 },
  { path: "javascript/branches.js", line: 148, lizard: 3, rerail: 2 },
  { path: "typescript/branches.ts", line: 169, lizard: 3, rerail: 2 },
  ...(["javascript/tokens.js", "typescript/tokens.ts"] as const).flatMap(path => [
    { path, line: 1, lizard: 2, rerail: 1 },
    { path, line: 5, lizard: 2, rerail: 1 },
    { path, line: 9, lizard: 2, rerail: 1 },
    { path, line: 13, lizard: 3, rerail: 1 },
    { path, line: 14, lizard: null, rerail: 3 },
    { path, line: 15, lizard: 1, rerail: null },
  ]),
  { path: "php/tokens.php", line: 5, lizard: 4, rerail: 1 },
  { path: "php/tokens.php", line: 11, lizard: 1, rerail: 3 },
  { path: "php/tokens.php", line: 18, lizard: 2, rerail: 1 },
  { path: "php/tokens.php", line: 26, lizard: 8, rerail: 2 },
  { path: "php/tokens.php", line: 28, lizard: null, rerail: 7 },
  { path: "blade/blocks.blade.php", line: 4, lizard: null, rerail: 7 },
  { path: "blade/blocks.blade.php", line: 22, lizard: null, rerail: 2 },
  { path: "rust/nesting.rs", line: 42, lizard: 3, rerail: null },
  { path: "rust/nesting.rs", line: 44, lizard: null, rerail: 3 },
  { path: "rust/tokens.rs", line: 4, lizard: 2, rerail: 1 },
  { path: "rust/tokens.rs", line: 12, lizard: 2, rerail: 1 },
  { path: "rust/tokens.rs", line: 17, lizard: 4, rerail: 3 },
  { path: "rust/tokens.rs", line: 24, lizard: 3, rerail: 1 },
  { path: "rust/tokens.rs", line: 30, lizard: 2, rerail: 3 },
  { path: "vue/tokens.vue", line: 5, lizard: 5, rerail: 1 },
  { path: "vue/tokens.vue", line: 11, lizard: 1, rerail: 3 },
];

const lizardRowSchema = z.object({ path: z.string().min(1), name: z.string().min(1), line: z.number().int().positive(), ccn: z.number().int().positive() });
const release = z
  .object({ tools: z.array(z.object({ name: z.string(), version: z.string() })) })
  .parse(JSON.parse(readFileSync(repositoryPath("packs", "complexity", "conformance.json"), "utf8")))
  .tools.find(tool => tool.name === "lizard")?.version;
const lizardRows = readFileSync(requirePackFixture(join(LIZARD, `lizard-${release}.tsv`)), "utf8")
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
function messages(compiled: NativeCompiledCodeRuleDsl, files: { path: string; source: string }[]): Record<string, string> {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: files.map(file => ({ language: languageFor(file.path) ?? "text", path: file.path, source: file.source })),
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  expect(result.violations.map(violation => violation.code).filter(code => code !== "CYCLOMATIC_COMPLEXITY")).toEqual([]);
  const keys = result.violations.map(violation => key(violation.path, violation.span.startLine + 1));
  expect(new Set(keys).size, "one finding per function").toBe(keys.length);
  return Object.fromEntries(result.violations.map(violation => [key(violation.path, violation.span.startLine + 1), violation.message]));
}

/** Findings by `path:line`, each mapped to the number its message carries. */
function findings(language: CorpusLanguage, compiled: NativeCompiledCodeRuleDsl, files: { path: string; source: string }[]): Record<string, number> {
  return Object.fromEntries(Object.entries(messages(compiled, files)).map(([at, text]) => [at, ccnOf(language, text)]));
}

const documents = loadPackRuleDocuments([repositoryPath("packs", "complexity")]).filter(document => document.rule.id.startsWith("complexity-"));
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
      Object.entries(expectedNumbers(language, path => compiled.inScope(path))).filter(([, ccn]) => ccn > LIMIT),
    );
    expect(Object.keys(expected).length, `${language} corpus has functions over the limit`).toBeGreaterThan(0);
    expect(findings(language, compiled, corpusFiles(language))).toEqual(expected);
  });

  it.each(LANGUAGES)("%s: measures every in-scope function as lizard does, apart from the listed deviations", language => {
    const compiled = compile(measuringJson(language));
    expect(findings(language, compiled, corpusFiles(language))).toEqual(expectedNumbers(language, path => compiled.inScope(path)));
  });

  it.each(LANGUAGES)("%s: leaves test files out even where lizard scores them over the limit", language => {
    const compiled = compile(ruleJson(language));
    const excluded = lizardRows.filter(row => row.path.startsWith(`${language}/`) && !compiled.inScope(row.path));
    expect(excluded.length, `${language} corpus has excluded test files`).toBeGreaterThan(0);
    for (const row of excluded) expect(row.ccn, key(row.path, row.line)).toBeGreaterThan(LIMIT);
    const files = corpusFiles(language);
    expect(findings(language, compiled, files.filter(file => excluded.some(row => row.path === file.path)))).toEqual({});
  });

  it.each([
    ["typescript", "typescript/__tests__/excluded.ts", ["src/widget.test.ts", "src/widget.spec.tsx", "src/__tests__/widget.mts"], "src/widget.cts"],
    ["javascript", "javascript/__tests__/excluded.js", ["src/widget.test.js", "src/widget.spec.jsx", "lib/widget.test.mjs"], "src/widget.cjs"],
    ["php", "php/tests/helpers.php", ["tests/Unit/helpers.php", "src/Tests/helpers.php", "app/HelpersTest.php", "app/HelpersTestCase.php", "spec/helpers.php"], "app/helpers.php"],
    ["blade", "blade/tests/excluded.blade.php", ["tests/views/excluded.blade.php", "Tests/excluded.blade.php"], "resources/views/excluded.blade.php"],
    ["rust", "rust/tests/excluded.rs", ["tests/excluded.rs", "crates/core/tests/excluded.rs", "src/tests.rs", "src/excluded_test.rs", "src/excluded_tests.rs"], "src/excluded.rs"],
    ["vue", "vue/__tests__/excluded.vue", ["src/__tests__/Widget.vue", "src/Widget.spec.vue", "src/Widget.test.vue"], "src/Widget.vue"],
  ] as const)("%s: excludes test files, and scores the same source elsewhere", (language, fixture, testPaths, sourcePath) => {
    const compiled = compile(ruleJson(language));
    const source = readFileSync(join(CORPUS, fixture), "utf8");
    for (const path of testPaths) {
      expect(compiled.inScope(path), path).toBe(false);
      expect(findings(language, compiled, [{ path, source }]), path).toEqual({});
    }
    expect(compiled.inScope(sourcePath)).toBe(true);
    expect(findings(language, compiled, [{ path: sourcePath, source }])).toEqual({ [key(sourcePath, 3)]: 8 });
  });

  // slapkit scores min(lizard's standard count, its modified count): a switch with a case, or a match, adds 1 however
  // many cases it has, so a flat dispatch table is not flagged. Per-case counting scored switchCountsOnce 8.
  const SWITCHES: Record<CorpusLanguage, { file: string; scores: Record<string, number> }> = {
    typescript: { file: "typescript/branches.ts", scores: { switchCountsOnce: 2, nestedSwitches: 3, switchDefaultOnly: 1, switchOverLimit: 8 } },
    javascript: { file: "javascript/branches.js", scores: { switchCountsOnce: 2, nestedSwitches: 3, switchDefaultOnly: 1, switchOverLimit: 8 } },
    python: { file: "python/match_case.py", scores: { match_counts_once: 2, nested_matches: 3, match_over_limit: 7, guarded_match_over_limit: 7 } },
    php: { file: "php/branches.php", scores: { switchCountsOnce: 2, nestedSwitches: 3, switchDefaultOnly: 1, switchOverLimit: 8, matchCountsOnce: 2, matchOverLimit: 7 } },
    blade: { file: "blade/functions.blade.php", scores: { tone: 2 } },
    rust: { file: "rust/branches.rs", scores: { match_counts_once: 2, nested_matches: 3, match_over_limit: 8 } },
    vue: { file: "vue/options.vue", scores: { level: 2 } },
  };

  it.each(LANGUAGES)("%s: a switch or match counts once however many cases it has, and only the functions over the limit are flagged", language => {
    const { file, scores } = SWITCHES[language];
    const at = (name: string) => {
      const row = lizardRows.find(candidate => candidate.path === file && candidate.name === name);
      if (row === undefined) throw new Error(`${file} has no ${name}`);
      expect(row.ccn, `lizard's ${name}`).toBe(scores[name]);
      return key(file, row.line);
    };
    const files = corpusFiles(language).filter(candidate => candidate.path === file);
    const measured = findings(language, compile(measuringJson(language)), files);
    for (const name of Object.keys(scores)) expect(measured[at(name)], name).toBe(scores[name]);
    const flagged = Object.keys(scores).filter(name => scores[name]! > LIMIT).map(at);
    expect(Object.keys(findings(language, compile(ruleJson(language)), files)).filter(found => Object.keys(scores).map(at).includes(found)).sort()).toEqual(flagged.sort());
  });

  /** `body`'s decisions, enough to put any function holding them over the limit (ccn 7). */
  const OVER: Record<CorpusLanguage, string> = {
    typescript: "if (a) {} if (a) {} if (a) {} if (a) {} if (a) {} if (a) {} return 1;",
    javascript: "if (a) {} if (a) {} if (a) {} if (a) {} if (a) {} if (a) {} return 1;",
    python: "if a: pass\n    if a: pass\n    if a: pass\n    if a: pass\n    if a: pass\n    if a: pass\n    return 1",
    php: "if ($a) {} if ($a) {} if ($a) {} if ($a) {} if ($a) {} if ($a) {} return 1;",
    blade: "if ($a) {} if ($a) {} if ($a) {} if ($a) {} if ($a) {} if ($a) {} return 1;",
    rust: "if a {} if a {} if a {} if a {} if a {} if a {} 1",
    vue: "if (a) {} if (a) {} if (a) {} if (a) {} if (a) {} if (a) {} return 1;",
  };

  // Functions at ccn 7, one per line of `%D%` (the decisions): each named as the steer should name it, and a callback
  // with no name at all (`undefined`) as "This function".
  const NAMED_TS = [
    "export function declared(a: number) { %D% }",
    "class Box {",
    "  method(a: number) { %D% }",
    "  get level() { const a = 1; %D% }",
    "  field = (a: number) => { %D% };",
    "}",
    "const assigned = (a: number) => { %D% };",
    "const handlers = { onKey: function (a: number) { %D% } };",
    "[1].map((a: number) => { %D% });",
  ];
  const NAMED_JS = NAMED_TS.map(line => line.replaceAll(": number", ""));
  const NAMED_PY = [
    "def declared(a):",
    "    %D%",
    "class Box:",
    "    def method(self, a):",
    "        %D%",
  ];
  const NAMED_PHP = [
    "<?php",
    "function declared($a) { %D% }",
    "class Box {",
    "    public function method($a) { %D% }",
    "    public static function make($a) { %D% }",
    "}",
    "$assigned = function ($a) { %D% };",
    "array_map(function ($a) { %D% }, [1]);",
  ];
  // The template's own logic (v-if, v-for) is no function's: only the script's functions are reported.
  const NAMED_VUE = [
    "<script>",
    "export function declared(a) { %D% }",
    "export default {",
    "  methods: {",
    "    method(a) { %D% },",
    "  },",
    "};",
    "const assigned = (a) => { %D% };",
    "[1].map((a) => { %D% });",
    "</script>",
    "<template>",
    '  <p v-for="a in items" :key="a" v-if="a && b || c">{{ a ? (b ? 1 : 2) : c && d ? 3 : e || f ? 4 : 5 }}</p>',
    "</template>",
  ];
  // A template's directives run in no function, and a function in an @php block or an echo is reported where it is.
  const NAMED_BLADE = [
    "<?php",
    "function declared($a) { %D% }",
    "?>",
    "@if ($a && $b || $c) @foreach ($items as $item) @if ($item) {{ $a ? 1 : ($b ? 2 : ($c ? 3 : 4)) }} @endif @endforeach @endif",
    "@php",
    "    $assigned = function ($a) { %D% };",
    "@endphp",
    "<p>{{ collect([1])->map(function ($a) { %D% }) }}</p>",
  ];
  const NAMED_RUST = [
    "fn declared(a: bool) -> i32 { %D% }",
    "struct Boxed;",
    "impl Boxed {",
    "    pub fn method(&self, a: bool) -> i32 { %D% }",
    "}",
    "trait Shape {",
    "    fn area(&self, a: bool) -> i32 { %D% }",
    "}",
  ];

  it.each([
    ["typescript", "src/names.ts", NAMED_TS, ["declared", "method", "level", "field", "assigned", "onKey", undefined]],
    ["javascript", "src/names.js", NAMED_JS, ["declared", "method", "level", "field", "assigned", "onKey", undefined]],
    ["python", "src/names.py", NAMED_PY, ["declared", "method"]],
    ["php", "src/names.php", NAMED_PHP, ["declared", "method", "make", "$assigned", undefined]],
    ["blade", "resources/views/names.blade.php", NAMED_BLADE, ["declared", "$assigned", undefined]],
    ["rust", "src/names.rs", NAMED_RUST, ["declared", "method", "area"]],
    ["vue", "src/Names.vue", NAMED_VUE, ["declared", "method", "assigned", undefined]],
  ] as const)("%s: names the function in the steer, or says 'This function' when it has no name", (language, path, lines, names) => {
    // A function starts on its `def` line in Python, and on the line holding its decisions in the one-line sources.
    const opens = (line: string) => (language === "python" ? /^\s*def /u.test(line) : line.includes("%D%"));
    const starts: number[] = [];
    const written: string[] = [];
    for (const line of lines) {
      if (opens(line)) starts.push(written.length + 1);
      const indent = /^ */u.exec(line)![0];
      written.push(...line.replace("%D%", OVER[language].replaceAll("\n    ", `\n${indent}`)).split("\n"));
    }
    const source = written.join("\n") + "\n";
    expect(starts.length).toBe(names.length);
    const expected = Object.fromEntries(starts.map((line, index) => [key(path, line), message(language, names[index], 7)]));
    expect(messages(compile(ruleJson(language)), [{ path, source }])).toEqual(expected);
  });

  it("scopes each rule to its own language's sources outside dependency and build output", () => {
    const scopes: Record<CorpusLanguage, { inside: string[]; outside: string[] }> = {
      python: { inside: ["app/service.py", "service.py"], outside: ["app/tests/service.py", "app/test_service.py", "app/service_test.py", "conftest.py", ".venv/lib/x.py", "app/service.ts"] },
      typescript: { inside: ["src/a.ts", "web/view.tsx", "lib/m.mts", "lib/c.cts"], outside: ["src/a.js", "node_modules/p/a.ts", "pkg/dist/a.ts", "src/generated/api.ts", "src/a.py"] },
      javascript: { inside: ["src/a.js", "web/view.jsx", "lib/m.mjs", "lib/c.cjs"], outside: ["src/a.ts", "node_modules/p/a.js", "pkg/dist/a.js", "src/a.min.js", "src/a.py"] },
      php: { inside: ["app/Models/User.php", "index.php"], outside: ["resources/views/home.blade.php", "vendor/acme/lib/Client.php", "node_modules/p/x.php", "src/generated/Api.php", "app/User.py"] },
      blade: { inside: ["resources/views/home.blade.php", "home.blade.php"], outside: ["app/Models/User.php", "vendor/acme/views/home.blade.php", "resources/views/home.vue"] },
      rust: { inside: ["src/main.rs", "crates/core/src/lib.rs"], outside: ["target/debug/build/out/bindings.rs", "src/generated/api.rs", "src/main.py"] },
      vue: { inside: ["src/App.vue", "components/Card.vue"], outside: ["node_modules/p/Card.vue", "dist/App.vue", ".nuxt/components/Card.vue", "src/App.ts"] },
    };
    for (const language of LANGUAGES) {
      const compiled = compile(ruleJson(language));
      for (const path of scopes[language].inside) expect(compiled.inScope(path), `${language} ${path}`).toBe(true);
      for (const path of scopes[language].outside) expect(compiled.inScope(path), `${language} ${path}`).toBe(false);
    }
  });
});
