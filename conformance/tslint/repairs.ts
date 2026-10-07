import type { ExpectedFailure } from "./lint-format.js";

/**
 * Minimal edits that make a fixture parse in tree-sitter-typescript, so a port can be scored on the code the
 * fixture is really about. Two kinds, kept apart because only one says something about real-world expressivity:
 *
 * - `fixture-artifact`: the fixture line is not valid TypeScript (TSLint's parser recovers; tsc reports a
 *   1xxx syntax/grammar error). Repairing it is harmless.
 * - `grammar-gap`: the line IS valid TypeScript (tsc reports no syntax/grammar error) but tree-sitter-typescript
 *   rejects or misparses it. Real code would hit the same wall, so a repair only shows what the port would do
 *   once the grammar is fixed.
 *
 * A repair edits one line of the markup-stripped source (1-based `line`, as `--rule` prints it). Either `text`
 * replaces the whole line, or the single occurrence of `from` becomes `to`. Every expected failure on an edited
 * line must keep its start column and its first characters, unless `dropExpected` says the failure is on the invalid
 * construct itself and is excluded from the repaired score.
 */
export interface FixtureRepair {
  variant: string;
  file: string;
  line: number;
  kind: "fixture-artifact" | "grammar-gap";
  text?: string;
  from?: string;
  to?: string;
  dropExpected?: true;
  why: string;
}

type Edit = Omit<FixtureRepair, "variant" | "file">;
const at = (variant: string, file: string, edits: Edit[]): FixtureRepair[] => edits.map(edit => ({ variant, file, ...edit }));
const ART = "fixture-artifact" as const;
const GAP = "grammar-gap" as const;

const typeInLine = (line: number, why: string): Edit => ({ line, kind: ART, from: ": ...number", to: ":    number", why });
const variableNameSpread = (variant: string, line = 30): FixtureRepair[] =>
  at(variant, "test.ts.lint", [typeInLine(line, "`name: ...number` is not a type (TS1110)")]);
const privatePrivate = (variant: string, line: number): FixtureRepair =>
  at(variant, "test.ts.lint", [{ line, kind: ART, from: "private private Helper", to: "private         Helper", why: "duplicate `private` (TS1028)" }])[0]!;
const throwArrow = (line: number, tail: string): Edit => ({
  line,
  kind: ART,
  from: `=> throw ${tail};`,
  to: `=>{throw ${tail}}`,
  why: "`() => throw x` is not valid TS (TS1005 '{' expected); same columns as a block body",
});
const importStar = (line: number): Edit => ({ line, kind: ART, from: "import * from ", to: "import        ", why: "`import * from` without `as` (TS1005)" });

export const FIXTURE_REPAIRS: Readonly<Record<string, readonly FixtureRepair[]>> = {
  "arrow-parens": [
    ...at("default", "test.ts.lint", [{ line: 6, kind: ART, text: "", why: "`var f = a: number => {};` is invalid (TS1005); the fixture comment says TSLint does not warn on it" }]),
    ...at("ban-single-arg-parens", "test.ts.lint", [{ line: 7, kind: ART, text: "", why: "same invalid line; TSLint does not warn on it" }]),
  ],
  "await-promise": at("for-await-of", "test.ts.lint", [{ line: 17, kind: ART, from: "Promise<string>)", to: "Promise<string>>)", why: "unclosed type argument list (TS1005 '>' expected)" }]),
  "completed-docs": [
    ...at("defaults", "edge-case/test.ts.lint", [{ line: 5, kind: ART, from: "{ ... }", to: "{     }", why: "`{ ... }` placeholder body (TS1128)" }]),
    ...at("defaults/edge-case", "test.ts.lint", [{ line: 5, kind: ART, from: "{ ... }", to: "{     }", why: "`{ ... }` placeholder body (TS1128)" }]),
    ...at("types", "test.ts.lint", [
      { line: 221, kind: ART, from: "BadEnumMember", to: "BadEnumMember,", why: "enum members without a comma (TS1357)" },
      { line: 343, kind: ART, from: "literal.prop = value; }", to: "literal.prop = value; },", why: "object literal members without a comma (TS1005)" },
    ]),
  ],
  "import-blacklist": at(".", "test.ts.lint", [
    { line: 10, kind: ART, text: "", why: "`from rxjs` without quotes (TS1141); not a blacklist hit in TSLint either" },
    { line: 11, kind: ART, text: "", why: "`require(lodash)` without quotes (TS1141)" },
  ]),
  "member-access": at("default", "test.ts.lint", [
    { line: 52, kind: ART, from: "abstract method", to: "         method", why: "abstract method with a body (TS1245); tree-sitter rejects the decorated abstract member" },
    { line: 53, kind: ART, from: "readonly static", to: "static readonly", why: "`readonly static` order (TS1029)" },
  ]),
  "newline-before-return": at("default", "test.tsx.lint", [
    { line: 6, kind: ART, from: "}) }</div>", to: "}) }</div>;", why: "adjacent top-level JSX expressions without separators (TS2657)" },
    { line: 10, kind: ART, from: "}) }</div>", to: "}) }</div>;", why: "adjacent top-level JSX expressions without separators" },
    { line: 13, kind: ART, from: "<span />;", to: "<span /> ", why: "`;` inside an arrow expression body (TS1005)" },
  ]),
  "newline-per-chained-call": at("default", "test.ts.lint", [
    { line: 10, kind: ART, from: "another() }", to: "another() })", why: "`.map(` never closed (TS1005 ')' expected)" },
  ]),
  "no-any": [
    ...at("default", "test.ts.lint", [{ line: 7, kind: ART, from: "fooArrow(", to: "fooArro=(", why: "`const fooArrow(a) => {` (TS1005); same columns" }]),
    ...at("ignore-rest-args", "test.ts.lint", [
      { line: 7, kind: ART, from: "fooArrow(", to: "fooArro=(", why: "`const fooArrow(a) => {` (TS1005); same columns" },
      { line: 19, kind: ART, from: "const function(", to: "function fnxxx(", why: "`const function(...)` (TS1389); same columns" },
    ]),
  ],
  "no-async-without-await": at(".", "test.ts.lint", [
    { line: 105, kind: ART, from: "async function foo;", to: "async function foo();", why: "`async function foo;` has no parameter list (TS1005); the expected failure stays on `async`" },
    { line: 130, kind: ART, from: "public const test", to: "public       test", why: "`const` class member (TS1248)" },
    { line: 134, kind: ART, from: "abstract async func", to: "abstract       func", why: "`abstract async` (TS1243)" },
  ]),
  "no-default-export": at(".", "test.ts.lint", [
    { line: 32, kind: GAP, from: "abstract class { }", to: "class          { }", why: "valid TS: `export default abstract class { }` is rejected by tree-sitter-typescript" },
  ]),
  "no-duplicate-switch-case": at(".", "test.ts.lint", [
    { line: 2, kind: ART, from: "switch (value):", to: "switch (value){", why: "`switch (x):` (TS1005 '{' expected)" },
    { line: 13, kind: ART, from: "};", to: "}};", why: "closes the repaired switch block" },
  ]),
  "no-empty-interface": at(".", "test.ts.lint", [
    { line: 9, kind: ART, from: "extends {}", to: "        {}", why: "empty `extends` list (TS1097); the failure still starts at the name" },
  ]),
  "no-implicit-dependencies": [
    ...at("whitelist", "test.ts.lint", [3, 5, 7, 9, 11, 13, 15].map(importStar)),
    ...at("whitelist-with-dev", "test.ts.lint", [3, 5, 7].map(importStar)),
  ],
  "no-invalid-template-strings": at(".", "test.ts.lint", [{ line: 13, kind: ART, from: "))))", to: "))) ", why: "one `)` too many (TS1128)" }]),
  "no-invalid-this": at("enabled", "test.ts.lint", [{ line: 22, kind: ART, from: "function(this) => {", to: "function(this)    {", why: "`function(this) => {` (TS1005)" }]),
  "no-sparse-arrays": at(".", "test.ts.lint", [
    { line: 8, kind: GAP, from: "(bar)", to: " bar ", why: "valid TS: parenthesised target in array destructuring assignment `[foo, , (bar)] = []` is rejected by tree-sitter" },
  ]),
  "no-string-literal": at(".", "test.ts.lint", [{ line: 25, kind: ART, from: "obj[]", to: "obj  ", why: "empty element access (TS1011)" }]),
  "no-string-throw": at(".", "test.ts.lint", [
    throwArrow(1, "'bla'"),
    throwArrow(3, "new Error('bla')"),
    throwArrow(5, "'string1' + 'string2' + 'string3'"),
    throwArrow(7, "'string' + 1"),
    throwArrow(9, "'string1' + 1 + {}"),
    throwArrow(11, "('string')"),
    throwArrow(13, "1 + 2 + ('string')"),
    throwArrow(17, "one.toString()"),
    throwArrow(19, "`some template string`"),
    throwArrow(22, "`template with ${someVariable} string`"),
  ]),
  "no-unnecessary-class": [
    ...at("allow-empty", "test.ts.lint", [{ line: 19, kind: ART, from: "private number hello", to: "private hello       ", why: "`private number hello` (TS1434)" }]),
    privatePrivate("allow-empty", 43),
    privatePrivate("allow-static-only", 34),
    privatePrivate("default", 34),
  ],
  "no-unnecessary-qualifier": at(".", "test.ts.lint", [
    { line: 14, kind: ART, from: "#if", to: "// ", why: "indented `#if typescript` is not a directive for TSLint's runner either; it is literal source (TS1127)" },
    { line: 26, kind: ART, from: "#endif", to: "//    ", why: "indented `#endif`, literal source" },
  ]),
  "object-literal-sort-keys": at("default", "test.ts.lint", [{ line: 107, kind: ART, from: "'11': 2", to: "'11': 2,", why: "missing comma (TS1005)" }]),
  "prefer-const": [
    ...at("default", "test.ts.lint", [{ line: 59, kind: ART, from: "[{1, 1}, {1, 1}]", to: "[[1, 1], [1, 1]]", why: "`{1, 1}` is not an object literal (TS1005)" }]),
    ...at("destructuring-all", "test.ts.lint", [{ line: 21, kind: ART, from: "[{1, 1}, {1, 1}]", to: "[[1, 1], [1, 1]]", why: "`{1, 1}` is not an object literal (TS1005)" }]),
    ...at("default", "spread.ts.lint", [
      { line: 6, kind: GAP, from: "[(d1), ([d2]), ...[d3 = 1, ...(d4)]]", to: "[ d1 ,  [d2] , ...[d3 = 1, ... d4 ]]", why: "valid TS: parenthesised destructuring-assignment targets are rejected by tree-sitter" },
      { line: 22, kind: GAP, from: "...(d5),", to: "... d5 ,", why: "valid TS: parenthesised object-rest assignment target" },
    ]),
  ],
  "prefer-for-of": at(".", "test.ts.lint", [{ line: 167, kind: ART, from: "i = 0, i < arr.length, ++i", to: "i = 0; i < arr.length; ++i", why: "`for (a, b, c)` with commas (TS1005)" }]),
  "prefer-readonly": at("default", "test.ts.lint", [
    { line: 68, kind: ART, from: "})();", to: "})() ", why: "`;` inside a parameter list (TS1005)" },
    { line: 90, kind: ART, from: "}", to: "},", why: "object literal members without a comma (TS1005)" },
    { line: 94, kind: ART, from: "}", to: "},", why: "object literal members without a comma" },
    { line: 115, kind: ART, from: "PostMinus+;", to: "PostMinus ;", why: "`x+;` (TS1109); TSLint's recovered tree has no write here, and it expects the member to be flagged" },
    { line: 116, kind: ART, from: "PostPlus+;", to: "PostPlus ;", why: "`x+;` (TS1109)" },
  ]),
  "prefer-switch": [
    ...at("default", "test.ts.lint", [{ line: 34, kind: ART, from: "|| && x", to: "|| 0&&x", why: "`|| &&` (TS1109); `0` stands in for TSLint's missing operand, so the chain still has only two simple cases" }]),
    ...at("min-cases-2", "test.ts.lint", [{ line: 33, kind: ART, from: "a.b) else", to: "a.b){}else", why: "`if (c) else` with no then-statement (TS1109)" }]),
  ],
  quotemark: [
    ...at("invalid-double", "test.ts.lint", [{ line: 2, kind: ART, text: "", why: "unterminated string (TS1002); TSLint's scanner sees no complete literal, so it expects nothing here" }]),
    ...at("invalid-single", "test.ts.lint", [{ line: 2, kind: ART, text: "", why: "unterminated string (TS1002); TSLint expects nothing here" }]),
    ...at("jsx-double", "test.tsx.lint", [{ line: 4, kind: ART, from: "></div>", to: "></div>);", why: "`export const a = (` never closed (TS1005)" }]),
    ...at("jsx-single", "test.tsx.lint", [{ line: 4, kind: ART, from: "></div>", to: "></div>);", why: "`export const a = (` never closed (TS1005)" }]),
  ],
  "restrict-plus-operands": [
    ...at("default", "test.ts.lint", [{ line: 40, kind: ART, from: '10), + 10', to: '10)  + 10', why: "`var a = x, + 10` (TS1134)" }]),
    ...at("esnext-bigint", "test.ts.lint", [{ line: 16, kind: ART, from: '"10"\t', to: '"10",', why: "missing comma (TS1005)" }]),
  ],
  "typedef": at("all", "test.ts.lint", [{ line: 33, kind: ART, from: "d = a - b;", to: "d = a - b; };", why: "function expression never closed (TS1005 '}' expected)" }]),
  "unnecessary-else": ["default", "allow-else-if"].flatMap(variant =>
    at(variant, "test.ts.lint", [
      { line: 86, kind: ART, from: 'throw "error;', to: 'throw "error";', why: "unterminated string (TS1002)" },
      { line: 92, kind: ART, from: 'throw "error;', to: 'throw "error";', why: "unterminated string (TS1002)" },
      { line: 115, kind: ART, from: 'throw "error;', to: 'throw "error";', why: "unterminated string (TS1002)" },
      { line: 120, kind: ART, from: "}", to: "} while (true);", why: "`do { }` without `while (...)` (TS1005)" },
    ]),
  ),
  "variable-name": [
    ...variableNameSpread("default"),
    ...variableNameSpread("allow-leading-trailing-underscore"),
    ...variableNameSpread("allow-leading-underscore"),
    ...variableNameSpread("allow-trailing-underscore"),
    ...variableNameSpread("allow-pascal-case"),
    ...at("allow-pascal-case", "test.ts.lint", [{ line: 40, kind: ART, from: "{bar:}", to: "{bar }", why: "`{bar:}` (TS1003)" }]),
    ...at("allow-snake-case", "test.ts.lint", [typeInLine(32, "`name: ...number` is not a type (TS1110)")]),
  ],
  whitespace: [
    ...at("all", "test.ts.lint", [
      { line: 99, kind: ART, from: "if (){", to: "if (x) {", dropExpected: true, why: "`if ()` has no condition (TS1109); the expected `{` failure sits on the invalid construct" },
      { line: 103, kind: ART, from: "if ()", to: "if (x)", why: "`if ()` has no condition (TS1109)" },
    ]),
    ...at("none", "test.ts.lint", [
      { line: 78, kind: ART, from: "if (){", to: "if (x){", why: "`if ()` has no condition (TS1109)" },
      { line: 82, kind: ART, from: "if ()", to: "if (x)", why: "`if ()` has no condition (TS1109)" },
    ]),
  ],
};

export interface RepairedSource {
  source: string;
  expected: ExpectedFailure[];
  dropped: number;
  errors: string[];
}

/** Applies a file's repairs, checking each expected failure on an edited line still starts on the same text. */
export function applyRepairs(source: string, expected: ExpectedFailure[], repairs: readonly FixtureRepair[]): RepairedSource {
  const lines = source.split("\n");
  const errors: string[] = [];
  const droppedLines = new Set<number>();
  for (const repair of repairs) {
    const index = repair.line - 1;
    const before = lines[index];
    if (before === undefined) {
      errors.push(`line ${repair.line} does not exist`);
      continue;
    }
    let after: string;
    if (repair.text !== undefined) after = repair.text;
    else if (repair.from !== undefined && repair.to !== undefined && before.split(repair.from).length === 2) after = before.replace(repair.from, repair.to);
    else {
      errors.push(`line ${repair.line}: \`${repair.from ?? ""}\` must occur exactly once in \`${before.trim()}\``);
      continue;
    }
    lines[index] = after;
    if (repair.dropExpected) {
      droppedLines.add(index);
      continue;
    }
    for (const failure of expected) {
      if (failure.startLine !== index) continue;
      const end = failure.endLine === failure.startLine ? failure.endCol : before.length;
      const width = Math.max(1, Math.min(3, end - failure.startCol));
      if (before.slice(failure.startCol, failure.startCol + width) !== after.slice(failure.startCol, failure.startCol + width)) {
        errors.push(`line ${repair.line}: repair moves the expected failure at column ${failure.startCol + 1}`);
      }
    }
  }
  const kept = expected.filter(failure => !droppedLines.has(failure.startLine));
  return { source: lines.join("\n"), expected: kept, dropped: expected.length - kept.length, errors };
}
