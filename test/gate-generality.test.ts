/**
 * Check 8 (generality, conformance/gates/generality.ts and policy-literals.ts): V1 Gate 3's tests from the rule
 * validation ladder, each a way a port overfits upstream's tests or a reading the gate must not accuse; the inputs
 * read from a tool's source.json; and the engine probe that finds the node a pattern context selects (probe.ts).
 */
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { z } from "zod";
import {
  anchoredAtStart,
  generalityGate,
  generalityReport,
  inputsFromSource,
  javascriptRegex,
  readFixtureTexts,
  type GeneralityInputs,
} from "../conformance/gates/generality.js";
import { NAME_KEYED_OBJECTS, policyLiterals, policyRegexes, regexWords, ROLE_BY_KEY, VOCABULARY, VOCABULARY_OBJECTS } from "../conformance/gates/policy-literals.js";
import { selectedText, withExpando } from "../conformance/gates/probe.js";
import { upstreamSourceSchema } from "../conformance/languages/pack.js";
import { codeRuleSchema, codeRuleValueSchema } from "../src/code-rule-schema.js";
import { rulePackFileSchema } from "../src/rule-packs.js";

function rule(body: Record<string, unknown>, extra: Record<string, unknown> = {}): { clauses: unknown[] } {
  return { clauses: [{ id: "c", forbid: { ast: { language: "typescript", rule: body, ...extra } } }] };
}

function python(body: Record<string, unknown>): { clauses: unknown[] } {
  return { clauses: [{ id: "c", forbid: { ast: { language: "python", rule: body } } }] };
}

function inputs(overrides: Partial<GeneralityInputs> = {}): GeneralityInputs {
  return {
    keywords: new Set(["new", "interface", "class", "var"]),
    commentVocabulary: new Set(["name", "param"]),
    implementation: new Set(["eval"]),
    sharedImplementation: new Set(["Infinity"]),
    fixtures: new Set(["foo", "eval", "Infinity", "address", "name", "I", "A", "someObj", "x", "MyClass", "callback", "property"]),
    fixtureTexts: [
      "const foo = 1;\nfoo();\neval(code);\nconst address = someObj.address;\nclass MyClass {}\nMyClass.create();\n",
      '/// <reference path="externalFormatter" />\nfunction run(callback, property, name) { return Infinity; }\n',
    ],
    implementationRegexes: [],
    selectedText: () => undefined,
    ...overrides,
  };
}

function flagged(document: { clauses?: unknown }, overrides: Partial<GeneralityInputs> = {}): string[] {
  return [...new Set(generalityGate(document, inputs(overrides)).literals.filter(literal => literal.class === "fixture-only").map(literal => literal.token))].sort();
}

describe("check 8: policy literals", () => {
  it("regexWords drops escapes, classes, groups, anchors and source escape sequences", () => {
    expect(regexWords("^(?:Infinity|NaN)$")).toEqual(["Infinity", "NaN"]);
    expect(regexWords("^(==|!=|===|!==)$")).toEqual([]);
    expect(regexWords("\\\\n|\\n")).toEqual([]);
    expect(regexWords("\\\\u00A0x|[a-z]+|(?<name>foo)\\k<name>")).toEqual(["x", "foo"]);
    expect(regexWords("\\$http|console\\.log")).toEqual(["http", "console", "log"]);
  });

  it("policyLiterals reads only fields that carry code, by role", () => {
    const literals = policyLiterals({
      clauses: [
        {
          id: "named-check",
          forbid: {
            ast: {
              language: "typescript",
              rule: {
                kind: "call_expression",
                any: [{ pattern: "eval($$$ARGS)" }, { pattern: { context: "class A { $N = $N }", selector: "public_field_definition" } }],
                has: { regex: "^(?:requests)$", stopBy: "end" },
                compare: { left: "A", op: "eq", right: "C", text: "tokens" },
                binds: { form: "method", bindings: { within: "B" } },
                resolves: { global: ["window"], imported: ["lodash.pull"] },
                arguments: { signature: ["url", "/", "verify"], bind: { V: "verify" } },
                value: { eq: "unsafe" },
              },
              languageOptions: { globals: { someObj: "readonly" } },
              utils: { helper: { matches: { other: { P: "$X" } } } },
            },
          },
          violation: { code: "CODE", message: "Do not call eval", anchor: "witness.span", fix: "safeEval($$$ARGS)" },
        },
        {
          id: "content",
          forbid: { regex: { source: "file.content", pattern: "\\bdebugger\\b|n$" } },
        },
      ],
    });
    const roles = Object.fromEntries(literals.map(literal => [literal.token, literal.role]));
    expect(roles).toEqual({
      eval: "pattern",
      class: "context",
      A: "context",
      requests: "regex",
      window: "value",
      lodash: "value",
      pull: "value",
      unsafe: "value",
      someObj: "value",
      safeEval: "pattern",
      debugger: "regex",
    });
    const context = literals.find(literal => literal.token === "A")!;
    expect(context.selector).toBe("public_field_definition");
    expect(context.language).toBe("typescript");
    expect(literals.find(literal => literal.token === "someObj")!.pointer).toBe("/clauses/0/forbid/ast/languageOptions/globals/someObj");
  });

  it("reads argument slots as names only in Python, where they are keyword names", () => {
    const slots = { pattern: "parseInt($$$)", arguments: { signature: ["value", "base"], absent: ["base"] } };
    expect(policyLiterals(rule(slots)).map(literal => literal.token)).toEqual(["parseInt"]);
    const call = { clauses: [{ id: "c", forbid: { ast: { language: "python", rule: { pattern: "get($$$)", arguments: { signature: ["url", "verify"], bind: { V: "verify" } } } } } }] };
    expect(policyLiterals(call).map(literal => literal.token).sort()).toEqual(["get", "verify"]);
  });

  it("does not judge the names of slots a signature only holds a position for", () => {
    // Ruff's B008 reads typing.NewType's `tp` at position 1 and B028 warnings.warn's `stacklevel` at position 2.
    const newtype = python({ kind: "call", arguments: { signature: ["name", "tp"], bind: { TP: "tp" }, mode: "syntactic" } });
    expect(policyLiterals(newtype).map(literal => literal.token)).toEqual(["tp"]);
    const warn = python({ kind: "call", arguments: { signature: ["message", "category", "stacklevel"], absent: ["stacklevel"], mode: "syntactic" } });
    expect(policyLiterals(warn).map(literal => literal.token)).toEqual(["stacklevel", "stacklevel"]);
    // Bound as the language binds a call, too: the engine reads only bound and absent slots, and a keyword reaches only
    // the slot of its own name (Bandit's b504 and b603 list ssl.wrap_socket's and Popen's whole parameter lists).
    const bound = python({ kind: "call", arguments: { signature: ["name", "tp"], bind: { TP: "tp" } } });
    expect(policyLiterals(bound).map(literal => literal.token)).toEqual(["tp"]);
  });

  it("a comment's parsed content is comment text through every relation, and its host is the comment again", () => {
    // ESLint's `/* globals a:off */`, as the eslint no-redeclare and no-unused-vars ports read it.
    const setting = { has: { kind: "setting", regex: "^off$" } };
    const directive = (has: Record<string, unknown>) => ({ grammar: "eslintDirective", value: { regex: "^globals?$" }, has });
    const document = rule({
      kind: "comment",
      parse: directive({
        kind: "item",
        all: [
          { not: setting },
          // host is the comment the parse reads; follows leaves it for another node, here another comment, parsed again.
          { host: { regex: "^/\\*\\s*globals", follows: { kind: "comment", parse: directive({ kind: "item", not: setting }) } } },
          { host: { follows: { kind: "identifier", regex: "^off$" } } },
        ],
      }),
    });
    const readsComments = Object.fromEntries(policyRegexes(document).map(regex => [regex.pointer, regex.commentText]));
    const at = "/clauses/0/forbid/ast/rule/parse";
    expect(readsComments).toEqual({
      [`${at}/value/regex`]: true,
      [`${at}/has/all/0/not/has/regex`]: true,
      [`${at}/has/all/1/host/regex`]: true,
      [`${at}/has/all/1/host/follows/parse/value/regex`]: true,
      [`${at}/has/all/1/host/follows/parse/has/not/has/regex`]: true,
      [`${at}/has/all/2/host/follows/regex`]: false,
    });
    // A parse of a node that is not a comment does not read comment text.
    const literal = rule({ kind: "string", parse: { grammar: "regex", has: { kind: "group", regex: "^off$" } } });
    expect(policyRegexes(literal).map(regex => regex.commentText)).toEqual([false]);
    expect(flagged(document, { fixtures: new Set(["off"]), commentVocabulary: new Set(["off", "globals"]) })).toEqual(["off"]);
  });
});

describe("check 8: generality", () => {
  it("fails a rule that hardcodes a name only upstream's tests use, and explains upstream's own names", () => {
    const result = generalityGate(rule({ pattern: "foo.eval($A)", any: [{ regex: "^Infinity$" }] }), inputs());
    expect(result.checks[0]!.id).toBe("generality:names");
    expect(result.checks[0]!.ok).toBe(false);
    expect(result.checks[0]!.detail).toMatch(/: foo$/u);
    expect(Object.fromEntries(result.literals.map(literal => [literal.token, literal.class]))).toEqual({
      foo: "fixture-only",
      eval: "implementation",
      Infinity: "shared-implementation",
    });
    expect(flagged(rule({ pattern: "new $C()", has: { regex: "^zzz$" } }))).toEqual([]);
  });

  it("judges context words inside the selected node and treats the rest as scaffolding", () => {
    const selectedText = (code: string, selector: string) => (selector === "public_field_definition" ? "$N = $N" : selector === "interface_declaration" ? code : undefined);
    // The review's overfit callable-types port: the fixture name I sits inside the selected declaration.
    expect(flagged(rule({ pattern: { context: "interface I { $$$A }", selector: "interface_declaration" } }), { selectedText })).toEqual(["I"]);
    // A in `class A { ... }` is outside the selected field definition.
    const scaffold = generalityGate(rule({ pattern: { context: "class A { $N = $N }", selector: "public_field_definition" } }), inputs({ selectedText }));
    expect(scaffold.literals.map(literal => [literal.token, literal.class])).toEqual([
      ["class", "language"],
      ["A", "scaffold"],
    ]);
    // Without the selected node every context word is judged.
    expect(flagged(rule({ pattern: { context: "class A { $N = $N }", selector: "public_field_definition" } }))).toEqual(["A"]);
  });

  it("treats documentation tags as syntax only where a rule matches comment text", () => {
    expect(flagged(rule({ pattern: "$O.name === $O.name" }))).toEqual(["name"]);
    expect(flagged(rule({ kind: "comment", regex: "@param\\s+name" }))).toEqual([]);
  });

  it("judges a regex by the test names it matches", () => {
    for (const pattern of ["^address$", "^[Aa]ddress$", "^addres+$", "^ad{2}ress$", "^\\x61ddress$", "^a(d)dress$"]) {
      expect(flagged(rule({ regex: pattern })), pattern).toEqual(["address"]);
    }
    // A regex matching many test words is syntax, and a one-letter fragment is not a name.
    expect(flagged(rule({ regex: "n$" }), { fixtures: new Set(["x", ...Array.from({ length: 20 }, (_, index) => `w${index}n`)]) })).toEqual([]);
    // Matching only names upstream's code explains is fine.
    expect(flagged(rule({ regex: "^(?:eval|Infinity)$" }))).toEqual([]);
    // A rule that declares a fixture name as a global hardcodes it too.
    expect(flagged(rule({ kind: "identifier" }, { languageOptions: { globals: { someObj: "readonly" } } }))).toEqual(["someObj"]);
  });

  it("does not read capture names or DSL enums as code names", () => {
    const document = rule(
      { compare: { left: "A", op: "eq", right: "C", text: "tokens" }, binds: { form: "method" }, value: { type: "array" } },
      { languageOptions: {}, utils: { u: { kind: "x" } } },
    );
    expect(generalityGate(document, inputs({ fixtures: new Set(["A", "C", "method", "array"]) })).literals).toEqual([]);
  });

  it("does not read a parsed grammar's error message as code", () => {
    // The percentFormat grammar's error node holds Ruff's own message; F509 tells its kind by the message's words.
    const unsupported = { kind: "error", value: { regex: "^unsupported format character" } };
    for (const node of [unsupported, { kind: "error", not: { value: unsupported.value } }, { all: [{ kind: "error" }, { value: unsupported.value }] }]) {
      const document = python({ kind: "string", parse: { grammar: "percentFormat", has: node } });
      expect(policyLiterals(document), JSON.stringify(node)).toEqual([]);
      expect(flagged(document, { fixtures: new Set(["unsupported", "character"]), fixtureTexts: ["# unsupported format character\n"] })).toEqual([]);
    }
    // A node the grammar reads from source still carries code, and so does a value outside any parsed tree.
    const conversion = python({ kind: "string", parse: { grammar: "percentFormat", has: { kind: "conversion", value: { regex: "^foo$" } } } });
    expect(policyLiterals(conversion).map(literal => literal.token)).toEqual(["foo"]);
    expect(policyLiterals(python({ kind: "error", value: { regex: "^foo$" } })).map(literal => literal.token)).toEqual(["foo"]);
  });

  it("judges the whole words a group of alternatives spells between word characters", () => {
    // Ruff's SIM201 spells "__eq__" | "__ne__" whole; the port's `^__(eq|ne)__$` spells the same names.
    const dunders = rule({ regex: "^__(eq|ne|lt)__$" });
    expect(policyLiterals(dunders).map(literal => literal.token)).toEqual(["__eq__", "__ne__", "__lt__"]);
    expect(flagged(dunders, { implementation: new Set(["__eq__", "__ne__", "__lt__"]), fixtures: new Set(["eq", "ne", "__eq__"]), fixtureTexts: ["eq(a, b)\nne = 1\n"] })).toEqual([]);
    expect(policyLiterals(rule({ regex: "^(?:get|set)attr$" })).map(literal => literal.token)).toEqual(["getattr", "setattr"]);
    // A group without a word beside it, or with a quantifier, spells its alternatives alone.
    expect(policyLiterals(rule({ regex: "^(eq|ne)$|__(lt)?__" })).map(literal => literal.token)).toEqual(["eq", "ne", "__", "lt"]);
    expect(flagged(rule({ regex: "^(foo|bar)Baz$" }), { fixtures: new Set(["foo", "fooBaz"]) })).toEqual(["fooBaz"]);
  });

  it("reads letters a start-anchored node regex runs into a quote as a string prefix, not a name", () => {
    // `^[rRuU]{0,2}['"]` reads a string literal's prefix; on whole test text it also meets the end of `"Ur"`.
    const texts = ['open("f", "Ur")\nopen("f", "rr")\nx = U"y"\n'];
    const fixtures = new Set(["open", "f", "Ur", "rr", "U", "y", "x"]);
    for (const pattern of ["^[rRuU]{0,2}['\"]", "(?i)^[ru]*['\"]", "^[bB]['\"]|^[rRuU]{0,2}['\"]"]) {
      expect(flagged(python({ kind: "string", regex: pattern }), { fixtures, fixtureTexts: texts }), pattern).toEqual([]);
    }
    // Unanchored, or reading the string's content past its quote, the regex still selects the name.
    expect(flagged(python({ kind: "string", regex: "[U]r['\"]" }), { fixtures, fixtureTexts: texts })).toEqual(["Ur"]);
    expect(flagged(python({ kind: "string", regex: "^['\"][U]r['\"]$" }), { fixtures, fixtureTexts: texts })).toEqual(["Ur"]);
    expect(flagged(python({ kind: "string", regex: "^zz|[U]r['\"]" }), { fixtures, fixtureTexts: texts })).toEqual(["Ur"]);
    expect(anchoredAtStart("(?i)^a|\\Ab")).toBe(true);
    expect(anchoredAtStart("^(?:a|b)|c")).toBe(false);
    expect(anchoredAtStart("^[|]x")).toBe(true);
  });

  it("judges every regex over upstream's test text, whatever words it spells", () => {
    // One-letter classes, character escapes and context characters cannot hide a name.
    for (const pattern of ["^f[o]o$", "^[Ff][Oo][Oo]$", "^\\x{66}oo$", "^[f]oo\\(", "^[[:lower:]]oo$"]) {
      expect(flagged(rule({ kind: "identifier", regex: pattern })), pattern).toEqual(["foo"]);
    }
    expect(flagged(rule({ kind: "type_identifier", regex: "^M[y]C[l]a[s]s$" }))).toEqual(["MyClass"]);
    expect(flagged({ clauses: [{ id: "c", forbid: { regex: { source: "file.content", pattern: "\\b[f]oo\\s*\\(" } } }] })).toEqual(["foo"]);
    expect(flagged(rule({ regex: "[Aa]ddress\\s*=" }))).toEqual(["address"]);
    expect(flagged(rule({ kind: "member_expression", regex: "^someObj\\.[Aa]ddress$" }))).toEqual(["address", "someObj"]);
    // Explained matches do not use up the limit, so one hidden name in a long alternation is still found.
    const keywords = ["break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else"];
    const text = `${keywords.join(" ")} foo\n`;
    expect(flagged(rule({ regex: `^(?:[f]oo|${keywords.join("|")})$` }), { keywords: new Set(keywords), fixtureTexts: [text] })).toEqual(["foo"]);
    // A regex that merely spans a fixture name, without depending on it, does not select it.
    expect(flagged(rule({ kind: "comment", regex: "^///\\s*<reference\\s+path=.*$" }))).toEqual([]);
  });

  it("accepts names that upstream's own selective regexes check", () => {
    const regexOnly = rule({ kind: "identifier", regex: "^[f]oo$" });
    expect(flagged(regexOnly, { implementationRegexes: [/^foo$/u] })).toEqual([]);
    // An upstream regex that matches most test words is syntax and explains nothing.
    expect(flagged(regexOnly, { implementationRegexes: [/\w/u] })).toEqual(["foo"]);
  });

  it("lets an upstream regex that spells names explain every test word it selects", () => {
    // Bandit's password regex selects many compound test names; each match depends on the word's letters.
    const candidates = /(^(pas+wo?r?d|pwd|token|secrete?)$|_(pas+wo?r?d|pwd|token|secrete?)_|_(pas+wo?r?d|pwd|token|secrete?)$)/iu;
    const compound = Array.from({ length: 12 }, (_, index) => `user${index}_password`);
    const fixtures = new Set(["password", "passwd", "secret", "pwd", ...compound]);
    const named = rule({ kind: "identifier", regex: `^(?:password|passwd|secret|${compound.join("|")})$` });
    expect(flagged(named, { fixtures, fixtureTexts: [[...fixtures].join("\n")], implementationRegexes: [candidates] })).toEqual([]);
    // A regex that spells no name and selects more than the limit is syntax-shaped and names nothing.
    const words = new Set(["password", ...compound]);
    expect(flagged(rule({ kind: "identifier", regex: "^password$" }), { fixtures: words, fixtureTexts: [[...words].join("\n")], implementationRegexes: [/^[a-z]+(?:\d+_[a-z]+)?$/u] })).toEqual([
      "password",
    ]);
  });

  it("treats documentation tags as syntax only in regexes that read comment text", () => {
    const tags = { commentVocabulary: new Set(["callback", "property", "name"]) };
    expect(flagged(rule({ kind: "identifier", regex: "^(?:callback|property|name)$" }), tags)).toEqual(["callback", "name", "property"]);
    expect(flagged(rule({ all: [{ kind: "comment" }, { not: { regex: "^(?:callback|property|name)$" } }] }), tags)).toEqual([]);
  });

  it("steps past an empty regex match before a character outside the BMP, as matchAll does", () => {
    // `^$` becomes an empty match at identifier boundaries; stepping one code unit would land inside the surrogate pair.
    const texts = ["x = 1  # 😀\nq = '𝑞𝑢𝑒𝑟𝑦'\nfoo()\n"];
    expect(flagged(rule({ regex: "^$|^foo$" }), { fixtureTexts: texts })).toEqual(["foo"]);
  });

  it("fails a regex it cannot run rather than passing it unjudged", () => {
    const result = generalityGate(rule({ regex: "(?x) ^ f o o $" }), inputs());
    expect(result.checks.map(check => [check.id, check.ok])).toEqual([
      ["generality:names", true],
      ["generality:regexes", false],
    ]);
  });

  it("runs a regex that can match empty past astral characters in upstream's test text", () => {
    // An empty match before an emoji used to resume inside its surrogate pair, where a unicode regex matches it again.
    const fixtureTexts = ["/**\n * @ts-expect-error \u{1F468}\u200D\u{1F469} */\nfoo();\n"];
    expect(flagged(rule({ regex: "^" }), { fixtureTexts })).toEqual([]);
    expect(flagged(rule({ regex: "^(?:foo)?$" }), { fixtureTexts })).toEqual(["foo"]);
  });

  it("selects no identifier longer than the window a syntax-shaped regex is tested in", () => {
    // ESLint's tests hold `"a".repeat(1e5)`; a window that cut the identifier misplaced it and called it selected.
    const word = "a".repeat(100_000);
    const fixtureTexts = [`a\n\n\n\n${word}`, `${word}: foo();`];
    expect(flagged(rule({ kind: "x", follows: { regex: "[^;{:]$" } }), { fixtures: new Set([word]), fixtureTexts })).toEqual([]);
  });

  it("does not name the test's typos where a regex writes out upstream's edit-distance check; a hardcoded typo it does", () => {
    /** A regex of every one-edit variant of each name, as a port writes upstream's edit distance 1 out (Next.js's no-typos). */
    const oneEditRegex = (names: string[]): string => {
      const variants = names.flatMap(name =>
        [...name]
          .flatMap((_, index) => [`${name.slice(0, index)}[A-Za-z0-9_$]${name.slice(index + 1)}`, name.slice(0, index) + name.slice(index + 1), `${name.slice(0, index)}[A-Za-z0-9_$]${name.slice(index)}`])
          .concat(`${name}[A-Za-z0-9_$]`),
      );
      return `^(?:${variants.join("|")})$`;
    };
    const typos = {
      implementation: new Set(["getStaticProps", "getServerSideProps"]),
      fixtures: new Set(["getStaticPropss", "getServurSideProps", "getStaticprops"]),
      fixtureTexts: ["export async function getStaticPropss() {}\nexport const getServurSideProps = 1;\nexport function getStaticprops() {}\n"],
    };
    expect(flagged(rule({ kind: "identifier", regex: oneEditRegex(["getStaticProps", "getServerSideProps"]) }), typos)).toEqual([]);
    expect(flagged(rule({ kind: "identifier", regex: "^(?:getStaticPropss|getServurSideProps)$" }), typos)).toEqual(["getServurSideProps", "getStaticPropss"]);
    // Typos the edit-distance check explains still count towards the syntax-pattern limit, so they cannot make a regex that
    // selects many test words look like a short list of names.
    const many = ["getStaticPropa", "getStaticPropb", "getStaticPropc", "getStaticPropd", "getStaticPrope", "getStaticPropf", "getStaticPropg", "getStaticProph"];
    const mixed = (typoWords: string[]) => ({
      implementation: new Set(["getStaticProps"]),
      fixtures: new Set([...typoWords, "axb", "bxc", "cxa"]),
      fixtureTexts: [[...typoWords, "axb", "bxc", "cxa"].map(word => `export const ${word} = 1;`).join("\n")],
    });
    const editsOrLetters = oneEditRegex(["getStaticProps"]).replace(/\)\$$/u, "|[a-c]x[a-c])$");
    expect(flagged(rule({ kind: "identifier", regex: editsOrLetters }), mixed(many))).toEqual([]);
    expect(flagged(rule({ kind: "identifier", regex: editsOrLetters }), mixed(many.slice(0, 2)))).toEqual(["axb", "bxc", "cxa"]);
    // One edit from an explained name is not enough: the regex must match the name's whole neighbourhood.
    expect(flagged(rule({ kind: "identifier", regex: "^getStaticProp[s]s$" }), typos)).toEqual(["getStaticPropss"]);
  });

  it("does not name the prefixes a regex refuses a word through; a regex spelling a prefix as a word does", () => {
    const pages = {
      implementation: new Set(["pages"]),
      fixtures: new Set(["page", "pages", "age", "index"]),
      fixtureTexts: ["const page = 1;\n// pages/index.js, the age of the page\n"],
    };
    // Next.js's no-typos: a path that contains `pages` and is not under `pages/api`, written without lookahead.
    const outsidePages =
      "^(?:[^p]|p(?:p|ap|agp|agep)*(?:[^pa]|a[^gp]|ag[^ep]|age[^sp]))*(?:p(?:p|ap|agp|agep)*(?:a|ag|age)?)?pages(?:[^p]|p(?:[^a]|$)|pa(?:[^g]|$)|pag(?:[^e]|$)|page(?:[^s]|$))";
    expect(flagged(rule({ kind: "program" }, { file: { path: { regex: outsidePages } } }), pages)).toEqual([]);
    expect(flagged(rule({ kind: "program" }, { file: { path: { regex: "^(?:page|pages)/" } } }), pages)).toEqual(["page"]);
    expect(flagged(rule({ kind: "program" }, { file: { path: { regex: "page[^x]" } } }), pages)).toEqual(["page"]);
  });

  it("judges a transform's replacement text only where it reaches a fix or a comparison", () => {
    // ESLint's max-params port labels a function's kind (`A=async|G=*|...`) and rewrites the labels into its message.
    const labelled = {
      language: "typescript",
      rule: { pattern: "function $F() {}" },
      transform: { H0: { replace: { source: "$F", replace: "^(.*)$", by: "Label=${1}" } }, H1: { replace: { source: "$H0", replace: "Label=", by: "Async " } } },
    };
    const tokens = (clause: Record<string, unknown>) =>
      [...new Set(policyLiterals({ clauses: [{ id: "c", ...clause }] }).map(literal => literal.token))].filter(token => token === "Label" || token === "Async").sort();
    expect(tokens({ forbid: { ast: labelled }, violation: { code: "C", anchor: "witness.span", message: "$H1 is too long" } })).toEqual([]);
    expect(tokens({ forbid: { ast: labelled }, violation: { code: "C", anchor: "witness.span", message: "m", fix: "$H1()" } })).toEqual(["Async", "Label"]);
    expect(tokens({ forbid: { ast: labelled }, violation: { code: "C", anchor: "witness.span", message: "m", fix: { template: "$H1", range: "witness.span" } } })).toEqual(["Async", "Label"]);
    const compared = { ...labelled, rule: { pattern: "function $F() {}", compare: { left: "H1", op: "eq", right: "F" } } };
    expect(tokens({ forbid: { ast: compared }, violation: { code: "C", anchor: "witness.span", message: "m" } })).toEqual(["Async", "Label"]);
  });

  it("names no word with a class-shaped regex; a regex that holds one of its letters or digits does", () => {
    // Bandit's b615 tests a revision's characters as upstream does (`all(c in string.hexdigits ...)`, 7 or more).
    const revision = { implementation: new Set(["revision"]), fixtures: new Set(["config", "revision", "abc1234567"]), fixtureTexts: ['config = {"revision": "abc1234567"}\n'] };
    expect(flagged(python({ kind: "string", regex: `^["']*[0-9A-Fa-f]{7,}["']*$` }), revision)).toEqual([]);
    expect(flagged(python({ kind: "string", regex: `^["']*[0-9A-Fa-f]{3}1234567["']*$` }), revision)).toEqual(["abc1234567"]);
    expect(flagged(rule({ kind: "identifier", regex: "^[Aa]ddress$" }))).toEqual(["address"]);
    expect(flagged(rule({ kind: "identifier", regex: "^[f][o][o]$" }))).toEqual(["foo"]);
  });
});

describe("check 8: Rust regexes as JavaScript", () => {
  it("reads escapes in pairs, so punctuation after an escaped backslash stays literal", () => {
    const regex = javascriptRegex(String.raw`^\\[^\n\r\\'"abfnrtv0-7xNuU]`);
    expect(regex).toBeDefined();
    expect(regex!.test(String.raw`\d`)).toBe(true);
    expect(regex!.test(String.raw`\n`)).toBe(false);
    expect(javascriptRegex(String.raw`a\'b`)!.test("a'b")).toBe(true);
  });

  it("translates the Rust syntax policies use, or declines", () => {
    expect(javascriptRegex("(?i)^address$")!.test("ADDRESS")).toBe(true);
    expect(javascriptRegex("(?P<n>a)\\k<n>")!.test("aa")).toBe(true);
    expect(javascriptRegex("^\\x{66}oo$")!.test("foo")).toBe(true);
    expect(javascriptRegex("^[[:upper:]]x$")!.test("Ax")).toBe(true);
    expect(javascriptRegex("^import[{*\\\"']")!.test("import{")).toBe(true);
    expect(javascriptRegex("(?s:.)x")!.test("\nx")).toBe(true);
    expect(javascriptRegex("(?x) a b")).toBeUndefined();
    expect(javascriptRegex("a(?i)b")).toBeUndefined();
  });

  it("spells the Rust syntax JavaScript writes differently", () => {
    expect(javascriptRegex("^\\pL\\PL$")!.test("a1")).toBe(true);
    expect(javascriptRegex("[\\pN_]+")!.test("1_")).toBe(true);
    expect(javascriptRegex("(?-u:\\b)todo(?-u:\\B)")!.test("a todoo")).toBe(true);
    expect(javascriptRegex("(?-u:\\b)todo(?-u:\\b)")!.test("todos")).toBe(false);
    expect(javascriptRegex("(?-u:\\w)")).toBeUndefined();
    expect(javascriptRegex("(?u)^a$")!.test("a")).toBe(true);
    expect(javascriptRegex("(?iu)^a$")!.test("A")).toBe(true);
    expect(javascriptRegex("(?u:a)(?iu:b)")!.test("aB")).toBe(true);
    expect(javascriptRegex("^}$")!.test("}")).toBe(true);
    expect(javascriptRegex("^a{2,3}}$")!.test("aaa}")).toBe(true);
    expect(javascriptRegex("^[}]$")!.test("}")).toBe(true);
    expect(javascriptRegex("^[]a]+$")!.test("]a")).toBe(true);
    expect(javascriptRegex("^[^]a]$")!.test("b")).toBe(true);
    expect(javascriptRegex("^[^]a]$")!.test("]")).toBe(false);
    expect(javascriptRegex("^[[:upper:]]\\x{66}$")!.test("Af")).toBe(true);
    // A braced escape's `}` belongs to the escape, outside a class as inside one.
    expect(javascriptRegex("^[\\p{XID_Start}_]\\p{XID_Continue}*$")!.test("a1")).toBe(true);
    expect(javascriptRegex("^\\P{Lowercase}*\\p{Uppercase}$")!.test("1A")).toBe(true);
    expect(javascriptRegex("^(?:\\P{M}[\\p{M}\\x{200C}]*){1,2}$")!.test("ab")).toBe(true);
    expect(javascriptRegex("^a\\x{200C}b$")!.test("a\u200Cb")).toBe(true);
  });

  it("runs Rust's class intersection and difference in unicodeSets mode, and declines symmetric difference", () => {
    const blankLine = javascriptRegex(String.raw`^[\s&&[^\xA0]]*\n[\s&&[^\xA0]]*$`);
    expect(blankLine!.test("  \n  ")).toBe(true);
    expect(blankLine!.test("\u00A0\n")).toBe(false);
    expect(blankLine!.test(" x\n")).toBe(false);
    expect(javascriptRegex("^[[a-z]--[aeiou]]$")!.test("b")).toBe(true);
    expect(javascriptRegex("^[[a-z]--[aeiou]]$")!.test("a")).toBe(false);
    // A bare range as an operand is Rust's spelling only; JavaScript's unicodeSets mode wants it bracketed.
    expect(javascriptRegex("^[a-z--[aeiou]]$")).toBeUndefined();
    expect(javascriptRegex("[[a-z]~~[aeiou]]")).toBeUndefined();
  });
});

describe("check 8: inputs from a tool's source facts", () => {
  const source = upstreamSourceSchema.parse({
    tool: "ruff",
    version: "0.16.9",
    repository: "https://github.com/astral-sh/ruff",
    commit: "0be08a206f9c3180afd3e93bcc792ed5cb1f4db1",
    tag: "0.16.9",
    configurations: ["default"],
    extensions: [".py", ".pyi"],
    keywords: ["def", "lambda"],
    commentVocabulary: ["coding", "ignore", "type"],
    testWords: ["eval", "foo", "getattr", "lambda", "password", "secret_token"],
    configurationWords: ["dummy_rgx"],
    sharedNames: ["getattr"],
    rules: {
      S105: {
        implementation: ["crates/ruff_linter/src/rules/flake8_bandit/rules/hardcoded_password_string.rs"],
        tests: ["crates/ruff_linter/resources/test/fixtures/flake8_bandit/S105.py"],
        deprecated: null,
        enabledBy: null,
        names: { implementation: ["eval"], shared: ["password"] },
        regexes: [{ source: "(^|_)(pas+wo?r?d|pass(phrase)?|pwd|token|secrete?)($|_)", flags: "i" }],
        unscored: [],
      },
    },
  });
  const texts = ["foo = eval\nsecret_token = 'x'\n"];

  it("reads the language's and the rule's names, the shared helpers' and the regex-selected test words", () => {
    const facts = inputsFromSource(source, "S105", texts, () => undefined)!;
    expect([...facts.keywords]).toEqual(["def", "lambda"]);
    expect([...facts.implementation]).toEqual(["eval"]);
    expect([...facts.sharedImplementation].sort()).toEqual(["getattr", "password"]);
    expect(facts.fixtures.has("secret_token")).toBe(true);
    expect(facts.implementationRegexes.map(regex => regex.flags)).toEqual(["i"]);
    // eval is the rule's, getattr a shared helper's, password and secret_token upstream's regex selects; foo is only tested.
    const document = python({ any: [{ pattern: "eval($A)" }, { pattern: "getattr($A, $B)" }, { regex: "^(?:password|secret_token|foo)$" }] });
    const result = generalityGate(document, facts);
    expect(result.checks[0]!.detail).toBe("matches 1 name(s) that occur in upstream's tests but not in upstream's code: foo");
    expect(generalityReport(result)).toEqual({
      words: { language: 0, implementation: 3, "shared-implementation": 1, "fixture-only": 1, scaffold: 0, unexplained: 0 },
      flagged: [{ token: "foo", pointer: "/clauses/0/forbid/ast/rule/any/2/regex", role: "regex", text: "^(?:password|secret_token|foo)$" }],
      uncheckedRegexes: [],
    });
  });

  it("counts configuration words as test words, but runs regexes only over test code", () => {
    const facts = inputsFromSource(source, "S105", texts, () => undefined)!;
    expect(facts.fixtures.has("dummy_rgx")).toBe(true);
    expect(facts.fixtureTexts).toEqual([...texts, "eval", "foo", "getattr", "lambda", "password", "secret_token"]);
    const result = generalityGate(python({ any: [{ regex: "^dummy_rgx$" }, { regex: "^du[m]my_rgx$" }] }), facts);
    expect(result.checks[0]!.detail).toBe("matches 1 name(s) that occur in upstream's tests but not in upstream's code: dummy_rgx");
  });

  it("has no inputs for a rule source.json does not record", () => {
    expect(inputsFromSource(source, "S106", texts, () => undefined)).toBeUndefined();
  });

  describe("readFixtureTexts", () => {
    let directory: string | undefined;
    afterEach(() => {
      if (directory !== undefined) rmSync(directory, { recursive: true, force: true });
      directory = undefined;
    });

    it("reads every copied file, nested ones included, in path order", () => {
      directory = mkdtempSync(join(tmpdir(), "rerail-fixtures-"));
      mkdirSync(join(directory, "b"));
      writeFileSync(join(directory, "b", "two.py"), "two\n");
      writeFileSync(join(directory, "a.py"), "one\n");
      writeFileSync(join(directory, "c.py"), "three\n");
      expect(readFixtureTexts(directory)).toEqual(["one\n", "two\n", "three\n"]);
    });
  });
});

describe("check 8: the engine probe for a pattern's context", () => {
  it("writes metavariables with the engine's expando as the engine reads a pattern", () => {
    expect(withExpando("foo($A, $$B, $$$C, $_, $$$)", "µ")).toBe("foo(µA, µµB, µµµC, µ_, µµµ)");
    // `$` before a lowercase letter or digit is not a metavariable, nor is a run of four.
    expect(withExpando("$a + $1 + $$$$x + $$", "µ")).toBe("$a + $1 + $$$$x + $$");
  });

  it("finds the selected node of a Python context, whose metavariables do not parse as Python", () => {
    expect(selectedText("foo($A, $$$B)", "call", "python")).toBe("foo($A, $$$B)");
    expect(selectedText("x = 'é'\nwith open($P) as $F:\n    $$$BODY", "call", "python")).toBe("open($P)");
    expect(selectedText("class A:\n    def $M(self, $$$P): ...", "function_definition", "python")).toBe("def $M(self, $$$P): ...");
  });

  it("finds the selected node of a TypeScript context as written", () => {
    expect(selectedText("class A { $N = $N }", "public_field_definition", "typescript")).toBe("$N = $N");
    expect(selectedText("const $X = <div>{$$$C}</div>;", "jsx_expression", "typescript")).toBeUndefined();
    expect(selectedText("function f($A) { return $A; }", "return_statement", "javascript")).toBe("return $A;");
  });

  it("selects nothing for a kind the grammar lacks, a language the engine does not read, or a context that does not parse", () => {
    expect(selectedText("foo()", "no_such_kind", "python")).toBeUndefined();
    expect(selectedText("foo()", "call_expression", "vue")).toBeUndefined();
    expect(selectedText("def (:", "call", "python")).toBeUndefined();
  });
});

type JsonSchema = { [key: string]: unknown };
const isJsonSchema = (value: unknown): value is JsonSchema => value !== null && typeof value === "object" && !Array.isArray(value);

/**
 * Every key of `codeRuleSchema` that can hold a string, and every object whose keys the document names rather than the
 * schema (a record), by the key that holds it (`<key>/*` for a record inside a record's values). A JSON value
 * (`codeRuleValueSchema`: option defaults, enums and conditions, configured values, violation variables, predicate
 * operands) can hold a string, and its object keys are data: it is not walked into.
 */
function auditedKeys(): { strings: Set<string>; objects: Set<string> } {
  const registry = z.registry<{ id: string }>();
  registry.add(codeRuleValueSchema, { id: "Value" });
  const root = z.toJSONSchema(codeRuleSchema, { metadata: registry, unrepresentable: "any", cycles: "ref", io: "input", reused: "inline" }) as JsonSchema;
  const definitions = isJsonSchema(root.$defs) ? root.$defs : {};
  const VALUE = "#/$defs/Value";
  const resolve = (value: unknown): JsonSchema | undefined => {
    let current = value;
    while (isJsonSchema(current) && typeof current.$ref === "string" && current.$ref !== VALUE)
      current = current.$ref === "#" ? root : definitions[current.$ref.replace("#/$defs/", "")];
    return isJsonSchema(current) ? current : undefined;
  };
  const parts = (schema: JsonSchema): unknown[] =>
    ["anyOf", "oneOf", "allOf", "prefixItems"].flatMap(key => (Array.isArray(schema[key]) ? (schema[key] as unknown[]) : [])).concat(schema.items === undefined ? [] : [schema.items]);
  const holdsString = (value: unknown, seen = new Set<JsonSchema>()): boolean => {
    const schema = resolve(value);
    if (schema === undefined || seen.has(schema)) return false;
    seen.add(schema);
    // A JSON value, an unconstrained value (an llm clause, a passthrough), a string, a string literal or enum.
    if (schema.$ref === VALUE || Object.keys(schema).length === 0) return true;
    if (schema.type === "string" || (Array.isArray(schema.type) && schema.type.includes("string"))) return true;
    if (typeof schema.const === "string" || (Array.isArray(schema.enum) && schema.enum.some(item => typeof item === "string"))) return true;
    return parts(schema).some(part => holdsString(part, seen));
  };
  const strings = new Set<string>();
  const objects = new Set<string>();
  const visited = new Set<JsonSchema>();
  const walk = (value: unknown, holder: string): void => {
    const schema = resolve(value);
    if (schema === undefined || schema.$ref === VALUE || visited.has(schema)) return;
    visited.add(schema);
    if (isJsonSchema(schema.properties)) {
      for (const [key, child] of Object.entries(schema.properties)) {
        if (holdsString(child)) strings.add(key);
        walk(child, key);
      }
    }
    if (isJsonSchema(schema.additionalProperties)) {
      objects.add(holder);
      walk(schema.additionalProperties, `${holder}/*`);
    }
    for (const part of parts(schema)) walk(part, holder);
  };
  walk(root, "");
  return { strings, objects };
}

describe("check 8: the key audit of core's policy schema", () => {
  const audited = auditedKeys();
  const sorted = (values: Iterable<string>): string[] => [...values].sort();

  it("classes every string-valued key of codeRuleSchema as carrying code or as DSL vocabulary, never both", () => {
    expect(audited.strings.size).toBeGreaterThan(80);
    for (const key of ["pattern", "context", "regex", "fix", "kind", "message"]) expect(audited.strings.has(key), key).toBe(true);
    expect(sorted(Object.keys(ROLE_BY_KEY).filter(key => key in VOCABULARY))).toEqual([]);
    expect(sorted([...Object.keys(ROLE_BY_KEY), ...Object.keys(VOCABULARY)])).toEqual(sorted(audited.strings));
  });

  it("classes every object whose keys a document names as code names or as names the document defines", () => {
    expect(sorted([...NAME_KEYED_OBJECTS].filter(key => key in VOCABULARY_OBJECTS))).toEqual([]);
    expect(sorted([...NAME_KEYED_OBJECTS, ...Object.keys(VOCABULARY_OBJECTS)])).toEqual(sorted(audited.objects));
  });

  it("reads an evidence predicate's operands as values, as it reads a value test's", () => {
    const predicate = (where: Record<string, unknown>) => ({ clauses: [{ id: "c", require_each: { evidence: { as: "E", where } } }] });
    const values = (where: Record<string, unknown>) => policyLiterals(predicate(where)).map(literal => [literal.token, literal.role]);
    expect(values({ ne: ["settings", "local_settings"] })).toEqual([
      ["settings", "value"],
      ["local_settings", "value"],
    ]);
    expect(values({ eq: ["settings", "local_settings"] })).toEqual(values({ ne: ["settings", "local_settings"] }));
  });
});

describe("check 8 reads every document of the python and typescript packs", () => {
  /** Documents whose patterns are metavariables only: they match by node kinds and relations, so they hardcode no word. */
  const NO_WORDS = new Set([
    "python-conformance:bandit-b101-assert-used",
    "python-conformance:ruff-f507-percent-format-positional-count-mismatch",
    "python-conformance:ruff-f823-undefined-local",
    "eslint-no-debugger",
    "eslint-no-unused-private-class-members",
  ]);
  const documents = ["python", "typescript"].flatMap(pack => {
    const directory = fileURLToPath(new URL(`../packs/${pack}/`, import.meta.url));
    const listed = rulePackFileSchema.parse(JSON.parse(readFileSync(join(directory, "pack.json"), "utf8")));
    return listed.rules.map(rule => ({ pack, id: rule.id, document: codeRuleSchema.parse(JSON.parse(readFileSync(join(directory, rule.document), "utf8"))) as { clauses?: unknown } }));
  });

  it("finds the words or regexes each document hardcodes, except in documents that hardcode none", () => {
    for (const pack of ["python", "typescript"]) expect(documents.filter(document => document.pack === pack).length, pack).toBeGreaterThan(0);
    const wordless = documents.filter(({ document }) => policyLiterals(document).length === 0 && policyRegexes(document).length === 0).map(({ id }) => id);
    expect(new Set(wordless)).toEqual(NO_WORDS);
  });

  it("runs every regex of every document as JavaScript, so generality:regexes judges them all", () => {
    for (const { id, document } of documents) {
      expect(generalityGate(document, inputs({ fixtureTexts: [] })).unchecked, id).toEqual([]);
    }
  });
});
