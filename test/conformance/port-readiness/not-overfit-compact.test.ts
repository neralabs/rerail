import assert from "node:assert/strict";
import { test } from "vitest";

import { anchoredAtStart, notOverfitGate, javascriptRegex, type NotOverfitInputs } from "../../../conformance/port-readiness/checks/not-overfit.js";
import { NESTING_RUN_LIMIT, REPEATED_SHARE_LIMIT, SIZE_FLAG_BYTES, compactGate } from "../../../conformance/port-readiness/checks/compact.js";
import { policyLiterals, policyRegexes, regexWords } from "../../../conformance/port-readiness/policy-literals.js";

const FLAGGED = SIZE_FLAG_BYTES + 1;

function rule(body: Record<string, unknown>, extra: Record<string, unknown> = {}): { clauses: unknown[] } {
  return { clauses: [{ id: "c", forbid: { ast: { language: "typescript", rule: body, ...extra } } }] };
}

function inputs(overrides: Partial<NotOverfitInputs> = {}): NotOverfitInputs {
  return {
    keywords: new Set(["new", "interface", "class", "var"]),
    commentVocabulary: new Set(["name", "param"]),
    implementation: new Set(["eval"]),
    sharedImplementation: new Set(["Infinity"]),
    fixtures: new Set(["foo", "eval", "Infinity", "address", "name", "I", "A", "someObj", "x", "MyClass", "callback", "property"]),
    fixtureTexts: [
      "const foo = 1;\nfoo();\neval(code);\nconst address = someObj.address;\nclass MyClass {}\nMyClass.create();\n",
      "/// <reference path=\"externalFormatter\" />\nfunction run(callback, property, name) { return Infinity; }\n",
    ],
    implementationRegexes: [],
    selectedText: () => undefined,
    ...overrides,
  };
}

function flagged(document: { clauses?: unknown }, overrides: Partial<NotOverfitInputs> = {}): string[] {
  return [...new Set(notOverfitGate(document, inputs(overrides)).literals.filter((literal) => literal.class === "fixture-only").map((literal) => literal.token))].sort();
}

test("regexWords drops escapes, classes, groups, anchors and source escape sequences", () => {
  assert.deepEqual(regexWords("^(?:Infinity|NaN)$"), ["Infinity", "NaN"]);
  assert.deepEqual(regexWords("^(==|!=|===|!==)$"), []);
  assert.deepEqual(regexWords("\\\\n|\\n"), []);
  assert.deepEqual(regexWords("\\\\u00A0x|[a-z]+|(?<name>foo)\\k<name>"), ["x", "foo"]);
  assert.deepEqual(regexWords("\\$http|console\\.log"), ["http", "console", "log"]);
});

test("policyLiterals reads only fields that carry code, by role", () => {
  const literals = policyLiterals({
    clauses: [{
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
    }, {
      id: "content",
      forbid: { regex: { source: "file.content", pattern: "\\bdebugger\\b|n$" } },
    }],
  });
  const roles = Object.fromEntries(literals.map((literal) => [literal.token, literal.role]));
  assert.deepEqual(roles, {
    eval: "pattern", class: "context", A: "context", requests: "regex", window: "value", lodash: "value", pull: "value",
    unsafe: "value", someObj: "value", safeEval: "pattern", debugger: "regex",
  });
  const context = literals.find((literal) => literal.token === "A")!;
  assert.equal(context.selector, "public_field_definition");
  assert.equal(context.language, "typescript");
  assert.equal(literals.find((literal) => literal.token === "someObj")!.pointer, "/clauses/0/forbid/ast/languageOptions/globals/someObj");
});

test("not-overfit fails a rule that hardcodes a name only upstream's tests use, and explains upstream's own names", () => {
  const result = notOverfitGate(rule({ pattern: "foo.eval($A)", any: [{ regex: "^Infinity$" }] }), inputs());
  assert.equal(result.checks[0]!.ok, false);
  assert.match(result.checks[0]!.detail, /: foo$/u);
  assert.deepEqual(Object.fromEntries(result.literals.map((literal) => [literal.token, literal.class])), {
    foo: "fixture-only", eval: "implementation", Infinity: "shared-implementation",
  });
  assert.deepEqual(flagged(rule({ pattern: "new $C()", has: { regex: "^zzz$" } })), []);
});

test("not-overfit judges context words inside the selected node and treats the rest as scaffolding", () => {
  const selectedText = (code: string, selector: string) =>
    selector === "public_field_definition" ? "$N = $N" : selector === "interface_declaration" ? code : undefined;
  // The review's overfit callable-types port: the fixture name I sits inside the selected declaration.
  assert.deepEqual(flagged(rule({ pattern: { context: "interface I { $$$A }", selector: "interface_declaration" } }), { selectedText }), ["I"]);
  // A in `class A { ... }` is outside the selected field definition.
  const scaffold = notOverfitGate(rule({ pattern: { context: "class A { $N = $N }", selector: "public_field_definition" } }), inputs({ selectedText }));
  assert.deepEqual(scaffold.literals.map((literal) => [literal.token, literal.class]), [["class", "language"], ["A", "scaffold"]]);
  // Without the selected node every context word is judged.
  assert.deepEqual(flagged(rule({ pattern: { context: "class A { $N = $N }", selector: "public_field_definition" } })), ["A"]);
});

test("not-overfit treats documentation tags as syntax only where a rule matches comment text", () => {
  assert.deepEqual(flagged(rule({ pattern: "$O.name === $O.name" })), ["name"]);
  assert.deepEqual(flagged(rule({ kind: "comment", regex: "@param\\s+name" })), []);
});

test("not-overfit judges a regex by the test names it matches", () => {
  for (const pattern of ["^address$", "^[Aa]ddress$", "^addres+$", "^ad{2}ress$", "^\\x61ddress$", "^a(d)dress$"]) {
    assert.deepEqual(flagged(rule({ regex: pattern })), ["address"], pattern);
  }
  // A regex matching many test words is syntax, and a one-letter fragment is not a name.
  assert.deepEqual(flagged(rule({ regex: "n$" }), { fixtures: new Set(["x", ...Array.from({ length: 20 }, (_, index) => `w${index}n`)]) }), []);
  // Matching only names upstream's code explains is fine.
  assert.deepEqual(flagged(rule({ regex: "^(?:eval|Infinity)$" })), []);
  // A rule that declares a fixture name as a global hardcodes it too.
  assert.deepEqual(flagged(rule({ kind: "identifier" }, { languageOptions: { globals: { someObj: "readonly" } } })), ["someObj"]);
});

test("not-overfit does not read capture names or DSL enums as code names", () => {
  const document = rule({ compare: { left: "A", op: "eq", right: "C", text: "tokens" }, binds: { form: "method" }, value: { type: "array" } }, {
    languageOptions: {}, utils: { u: { kind: "x" } },
  });
  assert.deepEqual(notOverfitGate(document, inputs({ fixtures: new Set(["A", "C", "method", "array"]) })).literals, []);
});

test("not-overfit reads argument slots as names only in Python, where they are keyword names", () => {
  const slots = { pattern: "parseInt($$$)", arguments: { signature: ["value", "base"], absent: ["base"] } };
  assert.deepEqual(policyLiterals(rule(slots)).map((literal) => literal.token), ["parseInt"]);
  const python = { clauses: [{ id: "c", forbid: { ast: { language: "python", rule: { pattern: "get($$$)", arguments: { signature: ["url", "verify"], bind: { V: "verify" } } } } } }] };
  assert.deepEqual(policyLiterals(python).map((literal) => literal.token).sort(), ["get", "verify"]);
});

function python(body: Record<string, unknown>): { clauses: unknown[] } {
  return { clauses: [{ id: "c", forbid: { ast: { language: "python", rule: body } } }] };
}

test("not-overfit does not judge the names of slots a signature only holds a position for", () => {
  // Ruff's B008 reads typing.NewType's `tp` at position 1 and B028 warnings.warn's `stacklevel` at position 2.
  const newtype = python({ kind: "call", arguments: { signature: ["name", "tp"], bind: { TP: "tp" }, mode: "syntactic" } });
  assert.deepEqual(policyLiterals(newtype).map((literal) => literal.token), ["tp"]);
  const warn = python({ kind: "call", arguments: { signature: ["message", "category", "stacklevel"], absent: ["stacklevel"], mode: "syntactic" } });
  assert.deepEqual(policyLiterals(warn).map((literal) => literal.token), ["stacklevel", "stacklevel"]);
  // Bound as the language binds a call, too: the engine reads only bound and absent slots, and a keyword reaches only
  // the slot of its own name (Bandit's b504 and b603 list ssl.wrap_socket's and Popen's whole parameter lists).
  const bound = python({ kind: "call", arguments: { signature: ["name", "tp"], bind: { TP: "tp" } } });
  assert.deepEqual(policyLiterals(bound).map((literal) => literal.token), ["tp"]);
});

test("not-overfit does not read a parsed grammar's error message as code", () => {
  // The percentFormat grammar's error node holds Ruff's own message; F509 tells its kind by the message's words.
  const unsupported = { kind: "error", value: { regex: "^unsupported format character" } };
  for (const node of [unsupported, { kind: "error", not: { value: unsupported.value } }, { all: [{ kind: "error" }, { value: unsupported.value }] }]) {
    const document = python({ kind: "string", parse: { grammar: "percentFormat", has: node } });
    assert.deepEqual(policyLiterals(document), [], JSON.stringify(node));
    assert.deepEqual(flagged(document, { fixtures: new Set(["unsupported", "character"]), fixtureTexts: ["# unsupported format character\n"] }), []);
  }
  // A node the grammar reads from source still carries code, and so does a value outside any parsed tree.
  const conversion = python({ kind: "string", parse: { grammar: "percentFormat", has: { kind: "conversion", value: { regex: "^foo$" } } } });
  assert.deepEqual(policyLiterals(conversion).map((literal) => literal.token), ["foo"]);
  assert.deepEqual(policyLiterals(python({ kind: "error", value: { regex: "^foo$" } })).map((literal) => literal.token), ["foo"]);
});

test("not-overfit judges the whole words a group of alternatives spells between word characters", () => {
  // Ruff's SIM201 spells "__eq__" | "__ne__" whole; the port's `^__(eq|ne)__$` spells the same names.
  const dunders = rule({ regex: "^__(eq|ne|lt)__$" });
  assert.deepEqual(policyLiterals(dunders).map((literal) => literal.token), ["__eq__", "__ne__", "__lt__"]);
  assert.deepEqual(flagged(dunders, { implementation: new Set(["__eq__", "__ne__", "__lt__"]), fixtures: new Set(["eq", "ne", "__eq__"]), fixtureTexts: ["eq(a, b)\nne = 1\n"] }), []);
  assert.deepEqual(policyLiterals(rule({ regex: "^(?:get|set)attr$" })).map((literal) => literal.token), ["getattr", "setattr"]);
  // A group without a word beside it, or with a quantifier, spells its alternatives alone.
  assert.deepEqual(policyLiterals(rule({ regex: "^(eq|ne)$|__(lt)?__" })).map((literal) => literal.token), ["eq", "ne", "__", "lt"]);
  assert.deepEqual(flagged(rule({ regex: "^(foo|bar)Baz$" }), { fixtures: new Set(["foo", "fooBaz"]) }), ["fooBaz"]);
});

test("not-overfit reads letters a start-anchored node regex runs into a quote as a string prefix, not a name", () => {
  // `^[rRuU]{0,2}['"]` reads a string literal's prefix; on whole test text it also meets the end of `"Ur"`.
  const texts = ['open("f", "Ur")\nopen("f", "rr")\nx = U"y"\n'];
  const fixtures = new Set(["open", "f", "Ur", "rr", "U", "y", "x"]);
  for (const pattern of ["^[rRuU]{0,2}['\"]", "(?i)^[ru]*['\"]", "^[bB]['\"]|^[rRuU]{0,2}['\"]"]) {
    assert.deepEqual(flagged(python({ kind: "string", regex: pattern }), { fixtures, fixtureTexts: texts }), [], pattern);
  }
  // Unanchored, or reading the string's content past its quote, the regex still selects the name.
  assert.deepEqual(flagged(python({ kind: "string", regex: "[U]r['\"]" }), { fixtures, fixtureTexts: texts }), ["Ur"]);
  assert.deepEqual(flagged(python({ kind: "string", regex: "^['\"][U]r['\"]$" }), { fixtures, fixtureTexts: texts }), ["Ur"]);
  assert.deepEqual(flagged(python({ kind: "string", regex: "^zz|[U]r['\"]" }), { fixtures, fixtureTexts: texts }), ["Ur"]);
  assert.equal(anchoredAtStart("(?i)^a|\\Ab"), true);
  assert.equal(anchoredAtStart("^(?:a|b)|c"), false);
  assert.equal(anchoredAtStart("^[|]x"), true);
});

test("not-overfit judges every regex over upstream's test text, whatever words it spells", () => {
  // One-letter classes, character escapes and context characters cannot hide a name.
  for (const pattern of ["^f[o]o$", "^[Ff][Oo][Oo]$", "^\\x{66}oo$", "^[f]oo\\(", "^[[:lower:]]oo$"]) {
    assert.deepEqual(flagged(rule({ kind: "identifier", regex: pattern })), ["foo"], pattern);
  }
  assert.deepEqual(flagged(rule({ kind: "type_identifier", regex: "^M[y]C[l]a[s]s$" })), ["MyClass"]);
  assert.deepEqual(flagged({ clauses: [{ id: "c", forbid: { regex: { source: "file.content", pattern: "\\b[f]oo\\s*\\(" } } }] }), ["foo"]);
  assert.deepEqual(flagged(rule({ regex: "[Aa]ddress\\s*=" })), ["address"]);
  assert.deepEqual(flagged(rule({ kind: "member_expression", regex: "^someObj\\.[Aa]ddress$" })), ["address", "someObj"]);
  // Explained matches do not use up the limit, so one hidden name in a long alternation is still found.
  const keywords = ["break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete", "do", "else"];
  const text = `${keywords.join(" ")} foo\n`;
  assert.deepEqual(flagged(rule({ regex: `^(?:[f]oo|${keywords.join("|")})$` }), { keywords: new Set(keywords), fixtureTexts: [text] }), ["foo"]);
  // A regex that merely spans a fixture name, without depending on it, does not select it.
  assert.deepEqual(flagged(rule({ kind: "comment", regex: "^///\\s*<reference\\s+path=.*$" })), []);
});

test("not-overfit judges a word in each text it occurs in, whatever order the texts come in", () => {
  // In the first text `ab \w+` matches whatever `foo` is spelled; in the second only `[f]oo` matches, at the same offset.
  const texts = ["ab foo\n", "cd foo\n"];
  const document = rule({ kind: "identifier", regex: "ab \\w+|[f]oo" });
  assert.deepEqual(flagged(document, { fixtureTexts: texts }), ["foo"]);
  assert.deepEqual(flagged(document, { fixtureTexts: [...texts].reverse() }), ["foo"]);
});

test("not-overfit accepts names that upstream's own selective regexes check", () => {
  const regexOnly = rule({ kind: "identifier", regex: "^[f]oo$" });
  assert.deepEqual(flagged(regexOnly, { implementationRegexes: [/^foo$/u] }), []);
  // An upstream regex that matches most test words is syntax and explains nothing.
  assert.deepEqual(flagged(regexOnly, { implementationRegexes: [/\w/u] }), ["foo"]);
});

test("not-overfit lets an upstream regex that spells names explain every test word it selects", () => {
  // Bandit's password regex selects many compound test names; each match depends on the word's letters.
  const candidates = /(^(pas+wo?r?d|pwd|token|secrete?)$|_(pas+wo?r?d|pwd|token|secrete?)_|_(pas+wo?r?d|pwd|token|secrete?)$)/iu;
  const compound = Array.from({ length: 12 }, (_, index) => `user${index}_password`);
  const fixtures = new Set(["password", "passwd", "secret", "pwd", ...compound]);
  const named = rule({ kind: "identifier", regex: `^(?:password|passwd|secret|${compound.join("|")})$` });
  assert.deepEqual(flagged(named, { fixtures, fixtureTexts: [[...fixtures].join("\n")], implementationRegexes: [candidates] }), []);
  // A regex that spells no name and selects more than the limit is syntax-shaped and names nothing.
  const words = new Set(["password", ...compound]);
  assert.deepEqual(flagged(rule({ kind: "identifier", regex: "^password$" }), { fixtures: words, fixtureTexts: [[...words].join("\n")], implementationRegexes: [/^[a-z]+(?:\d+_[a-z]+)?$/u] }), ["password"]);
});

test("not-overfit treats documentation tags as syntax only in regexes that read comment text", () => {
  const tags = { commentVocabulary: new Set(["callback", "property", "name"]) };
  assert.deepEqual(flagged(rule({ kind: "identifier", regex: "^(?:callback|property|name)$" }), tags), ["callback", "name", "property"]);
  assert.deepEqual(flagged(rule({ all: [{ kind: "comment" }, { not: { regex: "^(?:callback|property|name)$" } }] }), tags), []);
});

test("not-overfit steps past an empty regex match before a character outside the BMP, as matchAll does", () => {
  // `^$` becomes an empty match at identifier boundaries; stepping one code unit would land inside the surrogate pair.
  const texts = ["x = 1  # 😀\nq = '𝑞𝑢𝑒𝑟𝑦'\nfoo()\n"];
  assert.deepEqual(flagged(rule({ regex: "^$|^foo$" }), { fixtureTexts: texts }), ["foo"]);
});

test("a comment's parsed content is comment text through every relation, and its host is the comment again", () => {
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
  const readsComments = Object.fromEntries(policyRegexes(document).map((regex) => [regex.pointer, regex.commentText]));
  const at = "/clauses/0/forbid/ast/rule/parse";
  assert.deepEqual(readsComments, {
    [`${at}/value/regex`]: true,
    [`${at}/has/all/0/not/has/regex`]: true,
    [`${at}/has/all/1/host/regex`]: true,
    [`${at}/has/all/1/host/follows/parse/value/regex`]: true,
    [`${at}/has/all/1/host/follows/parse/has/not/has/regex`]: true,
    [`${at}/has/all/2/host/follows/regex`]: false,
  });
  // A parse of a node that is not a comment does not read comment text.
  const literal = rule({ kind: "string", parse: { grammar: "regex", has: { kind: "group", regex: "^off$" } } });
  assert.deepEqual(policyRegexes(literal).map((regex) => regex.commentText), [false]);
  assert.deepEqual(flagged(document, { fixtures: new Set(["off"]), commentVocabulary: new Set(["off", "globals"]) }), ["off"]);
});

test("not-overfit fails a regex it cannot run rather than passing it unjudged", () => {
  const result = notOverfitGate(rule({ regex: "(?x) ^ f o o $" }), inputs());
  assert.deepEqual(result.checks.map((check) => [check.id, check.ok]), [["no-fixture-only-names", true], ["regexes-judged", false]]);
});

test("javascriptRegex reads escapes in pairs, so punctuation after an escaped backslash stays literal", () => {
  const regex = javascriptRegex(String.raw`^\\[^\n\r\\'"abfnrtv0-7xNuU]`);
  assert.notEqual(regex, undefined);
  assert.equal(regex!.test(String.raw`\d`), true);
  assert.equal(regex!.test(String.raw`\n`), false);
  assert.equal(javascriptRegex(String.raw`a\'b`)!.test("a'b"), true);
});

test("javascriptRegex translates the Rust syntax policies use, or declines", () => {
  assert.equal(javascriptRegex("(?i)^address$")!.test("ADDRESS"), true);
  assert.equal(javascriptRegex("(?P<n>a)\\k<n>")!.test("aa"), true);
  assert.equal(javascriptRegex("^\\x{66}oo$")!.test("foo"), true);
  assert.equal(javascriptRegex("^[[:upper:]]x$")!.test("Ax"), true);
  assert.equal(javascriptRegex("^import[{*\\\"']")!.test("import{"), true);
  assert.equal(javascriptRegex("(?s:.)x")!.test("\nx"), true);
  assert.equal(javascriptRegex("(?x) a b"), undefined);
  assert.equal(javascriptRegex("a(?i)b"), undefined);
});

test("javascriptRegex spells the Rust syntax JavaScript writes differently", () => {
  assert.equal(javascriptRegex("^\\pL\\PL$")!.test("a1"), true);
  assert.equal(javascriptRegex("[\\pN_]+")!.test("1_"), true);
  assert.equal(javascriptRegex("(?-u:\\b)todo(?-u:\\B)")!.test("a todoo"), true);
  assert.equal(javascriptRegex("(?-u:\\b)todo(?-u:\\b)")!.test("todos"), false);
  assert.equal(javascriptRegex("(?-u:\\w)"), undefined);
  assert.equal(javascriptRegex("(?u)^a$")!.test("a"), true);
  assert.equal(javascriptRegex("(?iu)^a$")!.test("A"), true);
  assert.equal(javascriptRegex("(?u:a)(?iu:b)")!.test("aB"), true);
  assert.equal(javascriptRegex("^}$")!.test("}"), true);
  assert.equal(javascriptRegex("^a{2,3}}$")!.test("aaa}"), true);
  assert.equal(javascriptRegex("^[}]$")!.test("}"), true);
  assert.equal(javascriptRegex("^[]a]+$")!.test("]a"), true);
  assert.equal(javascriptRegex("^[^]a]$")!.test("b"), true);
  assert.equal(javascriptRegex("^[^]a]$")!.test("]"), false);
  assert.equal(javascriptRegex("^[[:upper:]]\\x{66}$")!.test("Af"), true);
  // A braced escape's `}` belongs to the escape, outside a class as inside one.
  assert.equal(javascriptRegex("^[\\p{XID_Start}_]\\p{XID_Continue}*$")!.test("a1"), true);
  assert.equal(javascriptRegex("^\\P{Lowercase}*\\p{Uppercase}$")!.test("1A"), true);
  assert.equal(javascriptRegex("^(?:\\P{M}[\\p{M}\\x{200C}]*){1,2}$")!.test("ab"), true);
  assert.equal(javascriptRegex("^a\\x{200C}b$")!.test("a\u200Cb"), true);
});

function share(document: { clauses?: unknown }): number {
  return compactGate(FLAGGED, document).metrics.repeatedShare;
}

test("compact passes small rules and flagged rules that write nothing out over and over", () => {
  assert.deepEqual(compactGate(100, rule({ kind: "debugger_statement" })).checks.map((check) => check.ok), [true]);
  // Two variants of one helper (break and continue) are a pairing, not repetition over and over.
  const variant = (keyword: string) => ({ all: [{ kind: `${keyword}_statement` }, { not: { has: { kind: "statement_identifier", stopBy: "end" } } }, { inside: { kind: "switch_body", stopBy: "end" } }] });
  const paired = rule({ any: [{ matches: "collects-break" }, { matches: "collects-continue" }] }, { utils: { "collects-break": variant("break"), "collects-continue": variant("continue") } });
  assert.equal(share(paired), 0);
  assert.deepEqual(compactGate(FLAGGED, paired).checks.map((check) => check.ok), [true, true, true]);
});

test("compact sends back a cartesian product written out cell by cell", () => {
  const cells = [];
  for (const receiver of ["window", "self", "globalThis", "top", "parent"]) {
    for (const method of ["setTimeout", "setInterval", "setImmediate", "requestIdleCallback", "queueMicrotask", "postMessage"]) {
      for (const enclosing of ["class_declaration", "function_declaration", "arrow_function", "method_definition", "program"]) {
        cells.push({ kind: "call_expression", regex: `^${receiver}\\.${method}\\b`, inside: { kind: enclosing, stopBy: "end" } });
      }
    }
  }
  const result = compactGate(FLAGGED, rule({ any: cells }));
  assert.ok(result.metrics.repeatedShare > 0.9, `repeated share ${result.metrics.repeatedShare}`);
  assert.equal(result.checks.find((check) => check.id === "no-combinatorial-expansion")!.ok, false);
});

test("compact sends back a product whose cells differ in shape but repeat their parts verbatim", () => {
  const receivers = ["window", "self", "globalThis", "top", "parent"];
  const methods = ["setTimeout", "setInterval", "setImmediate", "requestIdleCallback", "queueMicrotask", "postMessage"];
  const wrappers = (count: number) => ({ inside: { any: Array.from({ length: count }, (_, index) => ({ kind: `wrapper_kind_${index}` })), stopBy: "end" } });
  const argumentsOf = (count: number) => ({ has: { field: "arguments", any: Array.from({ length: count }, (_, index) => ({ kind: `argument_kind_${index}` })) } });
  const cells = receivers.flatMap((receiver, r) => methods.map((method, m) => ({ all: [{ regex: `^${receiver}\\.${method}$` }, wrappers(r + 1), argumentsOf(m + 1)] })));
  assert.ok(share(rule({ any: cells })) >= REPEATED_SHARE_LIMIT, `repeated share ${share(rule({ any: cells }))}`);
});

test("compact sends back a product hidden in a utility map or written in small cells", () => {
  const receivers = ["window", "self", "globalThis", "top", "parent"];
  const methods = ["setTimeout", "setInterval", "setImmediate", "requestIdleCallback", "queueMicrotask", "postMessage"];
  const cells = receivers.flatMap((receiver) => methods.map((method) => ({ kind: "call_expression", regex: `^${receiver}\\.${method}\\b`, inside: { kind: "program", stopBy: "end" } })));
  const utils = Object.fromEntries(cells.map((cell, index) => [`c${index}`, cell]));
  const inUtils = rule({ any: cells.map((_, index) => ({ matches: `c${index}` })) }, { utils });
  assert.ok(share(inUtils) >= REPEATED_SHARE_LIMIT, `utility map share ${share(inUtils)}`);
  const small = rule({ any: receivers.flatMap((receiver) => methods.map((method) => ({ pattern: `${receiver}.${method}($$$A)` }))) });
  assert.ok(share(small) >= REPEATED_SHARE_LIMIT, `small cells share ${share(small)}`);
});

test("compact does not treat a table of distinct cases as expansion", () => {
  const apis = ["util.isArray", "util.isBoolean", "fs.exists", "path._makeLong", "crypto.createCredentials", "domain.create", "os.tmpDir", "buffer.SlowBuffer"];
  const table = { clauses: apis.map((api, index) => ({
    id: `api-${index}`,
    forbid: { ast: { language: "typescript", rule: { kind: "call_expression", pattern: `${api}($$$A)` } } },
    violation: { anchor: "witness.span", code: "DEPRECATED", message: `'${api}' was deprecated; use its replacement instead.` },
  })) };
  assert.equal(share(table), 0);
});

test("compact finds repetition written out along one nesting path, through wrappers and beside other relations", () => {
  const chain = (depth: number, level: (inner: Record<string, unknown>) => Record<string, unknown>): Record<string, unknown> => {
    let node: Record<string, unknown> = { kind: "identifier" };
    for (let index = 0; index < depth; index += 1) node = level(node);
    return node;
  };
  const elseIf = chain(30, (inner) => ({ kind: "if_statement", has: { field: "alternative", kind: "else_clause", has: inner } }));
  const wrapped = chain(30, (inner) => ({ kind: "parenthesized_expression", all: [{ has: inner }] }));
  const beside = chain(50, (inner) => ({ kind: "parenthesized_expression", has: inner, not: { kind: "comment" } }));
  for (const [name, body] of Object.entries({ elseIf, wrapped, beside })) {
    const result = compactGate(FLAGGED, rule(body));
    assert.ok(result.metrics.nestingRun >= 30, `${name}: ${result.metrics.nestingRun}`);
    assert.equal(result.checks.find((check) => check.id === "no-written-out-repetition")!.ok, false, name);
  }
  // A(A(A(x))) is a node kind three times along one path, whether or not the last copy has a child.
  assert.equal(compactGate(FLAGGED, rule(chain(3, (inner) => ({ kind: "parenthesized_expression", has: inner })))).metrics.nestingRun, NESTING_RUN_LIMIT);
  // The all-list style, utilities and count hide nothing.
  const allList = chain(14, (inner) => ({ all: [{ kind: "if_statement" }, { has: { field: "alternative", all: [{ kind: "else_clause" }, { has: inner }] } }] }));
  assert.equal(compactGate(FLAGGED, rule(allList)).metrics.nestingRun, 14);
  const viaUtilities = rule({ matches: "level0" }, { utils: Object.fromEntries(Array.from({ length: 10 }, (_, index) => [`level${index}`, { kind: "parenthesized_expression", has: index < 9 ? { matches: `level${index + 1}` } : { kind: "identifier" } }])) });
  assert.equal(compactGate(FLAGGED, viaUtilities).metrics.nestingRun, 10);
  const viaCount = chain(6, (inner) => ({ kind: "parenthesized_expression", count: { has: inner, min: 1 } }));
  assert.equal(compactGate(FLAGGED, rule(viaCount)).metrics.nestingRun, 6);
  // Saying more about the same node is not nesting, nor are alternatives, nor what a node is not.
  assert.equal(compactGate(FLAGGED, rule({ kind: "call_expression", all: [{ kind: "call_expression" }], not: { kind: "call_expression" } })).metrics.nestingRun, 1);
  assert.equal(compactGate(FLAGGED, rule({ any: [{ kind: "block", has: { any: [{ kind: "block" }, { kind: "x" }] } }, { kind: "y" }] })).metrics.nestingRun, 2);
  // A recursive utility walks a tree; it is reported, not counted as written-out repetition.
  const recursive = rule({ matches: "walk" }, { utils: { walk: { any: [{ kind: "identifier" }, { kind: "parenthesized_expression", has: { matches: "walk" } }] } } });
  assert.equal(compactGate(FLAGGED, recursive).metrics.nestingRun, 1);
});

test("compact reports recursive utilities called by id or with parameters, per query", () => {
  const callForm = rule({ matches: "walk" }, { utils: { walk: { params: ["N"], rule: { any: [{ kind: "call_expression" }, { has: { matches: { walk: { N: "$N" } } } }] } } } });
  assert.deepEqual(compactGate(FLAGGED, callForm).metrics.recursiveUtils, ["walk"]);
  const separate = {
    clauses: [{
      id: "c",
      forbid: {
        select_when: {
          select: { ast: { language: "typescript", rule: { matches: "walk" }, utils: { walk: { any: [{ kind: "x" }, { has: { matches: "walk" } }] } } } },
          exists: { ast: { language: "typescript", rule: { matches: "walk" }, utils: { walk: { kind: "y" } } } },
        },
      },
    }],
  };
  assert.deepEqual(compactGate(FLAGGED, separate).metrics.recursiveUtils, ["walk"]);
});

test("a regex that can match empty runs past astral characters in upstream's test text", () => {
  // An empty match before an emoji used to resume inside its surrogate pair, where a unicode regex matches it again.
  const fixtureTexts = ["/**\n * @ts-expect-error \u{1F468}‍\u{1F469} */\nfoo();\n"];
  assert.deepEqual(flagged(rule({ regex: "^" }), { fixtureTexts }), []);
  assert.deepEqual(flagged(rule({ regex: "^(?:foo)?$" }), { fixtureTexts }), ["foo"]);
});

test("a syntax-shaped regex selects no identifier longer than the window it is tested in", () => {
  // ESLint's tests hold `"a".repeat(1e5)`; a window that cut the identifier misplaced it and called it selected.
  const word = "a".repeat(100_000);
  const fixtureTexts = [`a\n\n\n\n${word}`, `${word}: foo();`];
  assert.deepEqual(flagged(rule({ kind: "x", follows: { regex: "[^;{:]$" } }), { fixtures: new Set([word]), fixtureTexts }), []);
});

test("javascriptRegex runs Rust's class intersection and difference in unicodeSets mode, and declines symmetric difference", () => {
  const blankLine = javascriptRegex(String.raw`^[\s&&[^\xA0]]*\n[\s&&[^\xA0]]*$`);
  assert.equal(blankLine!.test("  \n  "), true);
  assert.equal(blankLine!.test(" \n"), false);
  assert.equal(blankLine!.test(" x\n"), false);
  assert.equal(javascriptRegex("^[[a-z]--[aeiou]]$")!.test("b"), true);
  assert.equal(javascriptRegex("^[[a-z]--[aeiou]]$")!.test("a"), false);
  assert.equal(javascriptRegex("[[a-z]~~[aeiou]]"), undefined);
});

test("javascriptRegex reads a Rust class operand as the union before the operator, applying operators left to right", () => {
  // Rust binds union tighter than `--` and `&&`; unicodeSets mode takes single operands, so the translation brackets them.
  const lowerStart = javascriptRegex(String.raw`^"[\p{L}_--\p{Lu}--\p{Lt}][\p{L}\p{N}_$--\p{Lu}--\p{Lt}]*"$`)!;
  assert.equal(lowerStart.source, String.raw`^"[[\p{L}_]--\p{Lu}--\p{Lt}][[\p{L}\p{N}_\$]--\p{Lu}--\p{Lt}]*"$`);
  assert.equal(lowerStart.flags, "v");
  for (const [text, matches] of [['"foo"', true], ['"_a$1"', true], ['"élan"', true], ['"Foo"', false], ['"fǅ"', false], ['"1a"', false], ['"$a"', false]] as const) {
    assert.equal(lowerStart.test(text), matches, text);
  }
  const vowelless = javascriptRegex("^[a-z--[aeiou]]$")!;
  assert.equal(vowelless.source, "^[[a-z]--[aeiou]]$");
  assert.equal(vowelless.test("b"), true);
  assert.equal(vowelless.test("a"), false);
  // A change of operator applies to everything before it.
  const mixed = javascriptRegex(String.raw`^[a-z--[aeiou]&&[a-f]]$`)!;
  assert.equal(mixed.source, "^[[[a-z]--[aeiou]]&&[a-f]]$");
  assert.deepEqual(["a", "b", "c", "d", "e", "f", "g"].filter((char) => mixed.test(char)), ["b", "c", "d", "f"]);
  // Punctuation unicodeSets mode reads as syntax stays literal, and negation applies to the whole expression.
  const punctuation = javascriptRegex(String.raw`^[^(|)\-[:digit:]--[5]]$`)!;
  assert.equal(punctuation.source, String.raw`^[^[\(\|\)\-[0-9]]--[5]]$`);
  assert.deepEqual(["(", "|", "-", "3", "5", "a"].filter((char) => punctuation.test(char)), ["5", "a"]);
});

/** A regex of every one-edit variant of each name, as a port writes upstream's edit distance 1 out (Next.js's no-typos). */
function oneEditRegex(names: string[]): string {
  const variants = names.flatMap((name) => [...name].flatMap((_, index) => [
    `${name.slice(0, index)}[A-Za-z0-9_$]${name.slice(index + 1)}`,
    name.slice(0, index) + name.slice(index + 1),
    `${name.slice(0, index)}[A-Za-z0-9_$]${name.slice(index)}`,
  ]).concat(`${name}[A-Za-z0-9_$]`));
  return `^(?:${variants.join("|")})$`;
}

test("a regex that writes out upstream's edit-distance check does not name the test's typos; one that hardcodes a typo does", () => {
  const typos = {
    implementation: new Set(["getStaticProps", "getServerSideProps"]),
    fixtures: new Set(["getStaticPropss", "getServurSideProps", "getStaticprops"]),
    fixtureTexts: ["export async function getStaticPropss() {}\nexport const getServurSideProps = 1;\nexport function getStaticprops() {}\n"],
  };
  assert.deepEqual(flagged(rule({ kind: "identifier", regex: oneEditRegex(["getStaticProps", "getServerSideProps"]) }), typos), []);
  assert.deepEqual(flagged(rule({ kind: "identifier", regex: "^(?:getStaticPropss|getServurSideProps)$" }), typos), ["getServurSideProps", "getStaticPropss"]);
  // Typos the edit-distance check explains still count towards the syntax-pattern limit, so they cannot make a regex that
  // selects many test words look like a short list of names.
  const many = ["getStaticPropa", "getStaticPropb", "getStaticPropc", "getStaticPropd", "getStaticPrope", "getStaticPropf", "getStaticPropg", "getStaticProph"];
  const mixed = (typoWords: string[]) => ({
    implementation: new Set(["getStaticProps"]),
    fixtures: new Set([...typoWords, "axb", "bxc", "cxa"]),
    fixtureTexts: [[...typoWords, "axb", "bxc", "cxa"].map((word) => `export const ${word} = 1;`).join("\n")],
  });
  const editsOrLetters = oneEditRegex(["getStaticProps"]).replace(/\)\$$/u, "|[a-c]x[a-c])$");
  assert.deepEqual(flagged(rule({ kind: "identifier", regex: editsOrLetters }), mixed(many)), []);
  assert.deepEqual(flagged(rule({ kind: "identifier", regex: editsOrLetters }), mixed(many.slice(0, 2))), ["axb", "bxc", "cxa"]);
  // One edit from an explained name is not enough: the regex must match the name's whole neighbourhood.
  assert.deepEqual(flagged(rule({ kind: "identifier", regex: "^getStaticProp[s]s$" }), typos), ["getStaticPropss"]);
});

test("a regex refusing a word through its prefixes does not name the prefixes; a regex spelling a prefix as a word does", () => {
  const pages = {
    implementation: new Set(["pages"]),
    fixtures: new Set(["page", "pages", "age", "index"]),
    fixtureTexts: ["const page = 1;\n// pages/index.js, the age of the page\n"],
  };
  // Next.js's no-typos: a path that contains `pages` and is not under `pages/api`, written without lookahead.
  const outsidePages = "^(?:[^p]|p(?:p|ap|agp|agep)*(?:[^pa]|a[^gp]|ag[^ep]|age[^sp]))*(?:p(?:p|ap|agp|agep)*(?:a|ag|age)?)?pages(?:[^p]|p(?:[^a]|$)|pa(?:[^g]|$)|pag(?:[^e]|$)|page(?:[^s]|$))";
  assert.deepEqual(flagged(rule({ kind: "program" }, { file: { path: { regex: outsidePages } } }), pages), []);
  assert.deepEqual(flagged(rule({ kind: "program" }, { file: { path: { regex: "^(?:page|pages)/" } } }), pages), ["page"]);
  assert.deepEqual(flagged(rule({ kind: "program" }, { file: { path: { regex: "page[^x]" } } }), pages), ["page"]);
});

test("not-overfit judges a transform's replacement text only where it reaches a fix or a comparison", () => {
  // ESLint's max-params port labels a function's kind (`A=async|G=*|...`) and rewrites the labels into its message.
  const labelled = {
    language: "typescript",
    rule: { pattern: "function $F() {}" },
    transform: { H0: { replace: { source: "$F", replace: "^(.*)$", by: "Label=${1}" } }, H1: { replace: { source: "$H0", replace: "Label=", by: "Async " } } },
  };
  const tokens = (clause: Record<string, unknown>) => [...new Set(policyLiterals({ clauses: [{ id: "c", ...clause }] }).map((literal) => literal.token))].filter((token) => token === "Label" || token === "Async").sort();
  assert.deepEqual(tokens({ forbid: { ast: labelled }, violation: { code: "C", anchor: "witness.span", message: "$H1 is too long" } }), []);
  assert.deepEqual(tokens({ forbid: { ast: labelled }, violation: { code: "C", anchor: "witness.span", message: "m", fix: "$H1()" } }), ["Async", "Label"]);
  assert.deepEqual(tokens({ forbid: { ast: labelled }, violation: { code: "C", anchor: "witness.span", message: "m", fix: { template: "$H1", range: "witness.span" } } }), ["Async", "Label"]);
  const compared = { ...labelled, rule: { pattern: "function $F() {}", compare: { left: "H1", op: "eq", right: "F" } } };
  assert.deepEqual(tokens({ forbid: { ast: compared }, violation: { code: "C", anchor: "witness.span", message: "m" } }), ["Async", "Label"]);
});

test("a class-shaped regex names no word; a regex that holds one of its letters or digits does", () => {
  // Bandit's b615 tests a revision's characters as upstream does (`all(c in string.hexdigits ...)`, 7 or more).
  const revision = { implementation: new Set(["revision"]), fixtures: new Set(["config", "revision", "abc1234567"]), fixtureTexts: ['config = {"revision": "abc1234567"}\n'] };
  assert.deepEqual(flagged(python({ kind: "string", regex: `^["']*[0-9A-Fa-f]{7,}["']*$` }), revision), []);
  assert.deepEqual(flagged(python({ kind: "string", regex: `^["']*[0-9A-Fa-f]{3}1234567["']*$` }), revision), ["abc1234567"]);
  assert.deepEqual(flagged(rule({ kind: "identifier", regex: "^[Aa]ddress$" })), ["address"]);
  assert.deepEqual(flagged(rule({ kind: "identifier", regex: "^[f][o][o]$" })), ["foo"]);
});
