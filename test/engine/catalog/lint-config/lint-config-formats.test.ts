import { describe, expect, it } from "vitest";
import { readIni, type IniDialect, type IniSections } from "../../../../src/engine/catalog/lint-config/formats/ini-format.js";
import { LintConfigFormatError, parseJsonc, readFormat } from "../../../../src/engine/catalog/lint-config/formats/lint-config-formats.js";
import { isNeonArray, neonEntry, parseNeon, type NeonValue } from "../../../../src/engine/catalog/lint-config/formats/neon-format.js";
import { parseXml, xmlChildren } from "../../../../src/engine/catalog/lint-config/formats/xml-format.js";

describe("readFormat", () => {
  it("gives the value read, or a format reader's refusal as why the file is not read", () => {
    expect(readFormat(() => parseJsonc("{ \"a\": 1 }"))).toEqual({ ok: true, value: { a: 1 } });
    expect(readFormat(() => parseJsonc("{ /* open"))).toEqual({ ok: false, why: "a /* comment is not closed" });
  });

  it("lets any other error through: it is a bug, not a file Rerail cannot read", () => {
    const bug = new TypeError("a bug");
    expect(() => readFormat(() => {
      throw bug;
    })).toThrow(bug);
  });
});

describe("parseJsonc", () => {
  it("reads comments and trailing commas, leaving strings alone", () => {
    const text = [
      "\uFEFF{",
      "  // a line comment, with a trailing comma after it",
      "  \"rules\": { \"no-console\": \"off\", /* a block, comment */ },",
      "  \"url\": \"https://example.com/a,]\", // the string keeps its // and ,]",
      "  \"list\": [1, 2, /* three */],",
      "  \"quoted\": \"a \\\"b\\\" /* c */\",",
      "  \"empty\": [],",
      "}",
    ].join("\n");
    expect(parseJsonc(text)).toEqual({ rules: { "no-console": "off" }, url: "https://example.com/a,]", list: [1, 2], quoted: "a \"b\" /* c */", empty: [] });
  });

  it.each([
    ["a line comment at the end of the file", "[1, 2] // the end", [1, 2]],
    ["a comma followed by a string is no trailing comma", "[\"a\",\"b\"]", ["a", "b"]],
    ["a comma inside a string is kept", "{\"a\": \",\"}", { a: "," }],
    ["a trailing comma before a closing brace", "{\"a\": 1,\n}", { a: 1 }],
  ])("reads %s", (_, text, value) => {
    expect(parseJsonc(text)).toEqual(value);
  });

  it.each([
    ["YAML", "rules:\n  no-console: off\n", /^not JSON: /u],
    ["an unclosed block comment", "{ /* open ", /^a \/\* comment is not closed$/u],
    ["a string open at the end of the file", "{ \"a\": \"open }", /^a string is not closed$/u],
    ["a string open at the end of its line", "{ \"a\": \"open\n\" }", /^a string is not closed$/u],
    ["a slash that starts no comment", "{ \"a\": 1 / 2 }", /^not JSON: /u],
    ["two commas in a row", "[1,,]", /^not JSON: /u],
  ])("refuses %s", (_, text, message) => {
    expect(() => parseJsonc(text)).toThrow(LintConfigFormatError);
    expect(() => parseJsonc(text)).toThrow(message);
  });
});

const BANDIT: IniDialect = { delimiters: ["=", ":"], lowerCaseOptions: true, interpolation: true };
const SQLFLUFF: IniDialect = { delimiters: ["="], lowerCaseOptions: false, interpolation: false };
/** Sections as plain objects, for comparing. */
const plainIni = (sections: IniSections): unknown => Object.fromEntries([...sections].map(([name, options]) => [name, Object.fromEntries(options)]));

describe("readIni", () => {
  it("reads sections, comments, values over several lines and [DEFAULT] as configparser does", () => {
    const text = [
      "# a comment",
      "; another",
      "",
      "[DEFAULT]",
      "Shared = base",
      "[bandit]",
      "",
      "Targets: src",
      "skips = B101,",
      "    B102",
      "",
      "  # an indented comment inside a value is skipped",
      "    B103",
      "",
      "shared = own",
      "[DEFAULT]",
      "late = 1",
      "[other]] trailing",
      "empty =",
    ].join("\n");
    expect(plainIni(readIni(text, BANDIT))).toEqual({
      bandit: { shared: "own", late: "1", targets: "src", skips: "B101,\nB102\n\nB103" },
      "other]": { shared: "base", late: "1", empty: "" },
    });
  });

  it.each([
    ["CR and CRLF line ends", "[s]\r\na = 1\rb = 2\n", BANDIT, { s: { a: "1", b: "2" } }],
    ["a value continued deeper than its indented option", "[s]\n  a = 1\n    b\n  c = 2", BANDIT, { s: { a: "1\nb", c: "2" } }],
    ["option names in their case, = alone a delimiter, and % as text", "[sqlfluff]\nDialect = ansi\nkey: value = x\nrules = 50%", SQLFLUFF, { sqlfluff: { Dialect: "ansi", "key: value": "x", rules: "50%" } }],
  ])("reads %s", (_, text, dialect, sections) => {
    expect(plainIni(readIni(text, dialect))).toEqual(sections);
  });

  it.each([
    ["a byte order mark", "\uFEFF[s]\n", BANDIT, "it starts with a byte order mark, which configparser reads as text"],
    ["a section set twice", "[s]\n[s]\n", BANDIT, "line 2: section [s] is set twice"],
    ["an option before any section", "\na = 1\n", BANDIT, "line 2: an option comes before any [section]"],
    ["a line that is no option", "[s]\njust words\n", BANDIT, "line 2: \"just words\" is not a section header or a \"key = value\" option"],
    ["an option without a name", "[s]\n= 1\n", BANDIT, "line 2: \"= 1\" is not a section header or a \"key = value\" option"],
    ["a delimiter the dialect does not take", "[s]\nkey: value\n", SQLFLUFF, "line 2: \"key: value\" is not a section header or a \"key = value\" option"],
    ["an option set twice, its case folded", "[s]\nA = 1\na = 2\n", BANDIT, "line 3: option a in [s] is set twice"],
    ["a [DEFAULT] option set twice across its sections", "[DEFAULT]\na = 1\n[DEFAULT]\na = 2\n", BANDIT, "line 4: option a in [DEFAULT] is set twice"],
    ["a value configparser interpolates", "[s]\na = 50%\n", BANDIT, "the value of a has a \"%\", which configparser interpolates and Rerail does not"],
  ])("refuses %s", (_, text, dialect, message) => {
    expect(() => readIni(text, dialect)).toThrow(LintConfigFormatError);
    expect(() => readIni(text, dialect)).toThrow(message);
  });
});

describe("parseXml", () => {
  it("reads elements, attributes, text, CDATA, references and namespaces", () => {
    const root = parseXml([
      "\uFEFF<?xml version=\"1.0\"?>",
      "<!-- a comment -->",
      "<psalm xmlns=\"https://getpsalm.org/schema/config\" xmlns:xi=\"http://www.w3.org/2001/XInclude\" errorLevel='3'>",
      "  <issueHandlers><ForbiddenCode errorLevel=\"suppress\"/></issueHandlers>",
      "  <text>a &amp; b &#x41;&#66;&lt;&gt;&quot;&apos;<![CDATA[<raw &x>]]><?pi skipped?><!-- skipped -->.</text >",
      "  <xi:include href=\"other.xml\"/>",
      "  <plain xmlns=\"\"><inner/></plain>",
      "</psalm>",
      "<!-- after -->",
      "<?pi after?>",
      "",
    ].join("\n"));
    expect(root.localName).toBe("psalm");
    expect(root.namespace).toBe("https://getpsalm.org/schema/config");
    expect(root.attributes.get("errorLevel")).toBe("3");
    expect(root.children.map(child => child.localName)).toEqual(["issueHandlers", "text", "include", "plain"]);
    expect(root.children[0]?.children[0]?.namespace).toBe("https://getpsalm.org/schema/config");
    expect(root.children[1]?.text).toBe("a & b AB<>\"'<raw &x>.");
    expect(root.children[2]?.namespace).toBe("http://www.w3.org/2001/XInclude");
    expect(root.children[3]?.namespace).toBeUndefined();
    expect(root.children[3]?.children[0]?.namespace).toBeUndefined();
    expect(root.children[0]?.children[0]?.line).toBe(4);
    expect(xmlChildren(root, "text").map(child => child.line)).toEqual([5]);
  });

  it("reads attribute values with references, each whitespace character a space", () => {
    const root = parseXml("<a b = \"x&#10;&#x9;y\" c='1&lt;2' d=\"t\tu\nv\r\"/>");
    expect(Object.fromEntries(root.attributes)).toEqual({ b: "x\n\ty", c: "1<2", d: "t u v " });
    expect(root.children).toEqual([]);
    expect(root.text).toBe("");
  });

  it("reads elements nested 64 deep, and refuses one deeper", () => {
    expect(parseXml(`${"<a>".repeat(64)}${"</a>".repeat(64)}`).localName).toBe("a");
    expect(() => parseXml(`${"<a>".repeat(65)}${"</a>".repeat(65)}`)).toThrow("not XML Rerail reads: elements nested over 64 deep (line 1)");
  });

  it.each([
    ["a document type declaration", "<!DOCTYPE ruleset [<!ENTITY a \"b\">]><ruleset/>", "a document type declaration is not read (line 1)"],
    ["an empty document", "", "no root element (line 1)"],
    ["text for a document", "\ntext", "no root element (line 2)"],
    ["a second root element", "<ruleset/>\n<other/>", "content after the root element (line 2)"],
    ["an unterminated comment", "<!-- open", "an unterminated comment (line 1)"],
    ["an unterminated comment in content", "<r>\n<!-- open</r>", "an unterminated comment (line 2)"],
    ["an unterminated processing instruction", "<?xml open", "an unterminated processing instruction (line 1)"],
    ["an unterminated processing instruction in content", "<r><?pi</r>", "an unterminated processing instruction (line 1)"],
    ["a start tag without a name", "< r/>", "a name was expected (line 1)"],
    ["an end tag for a root", "</r>", "a name was expected (line 1)"],
    ["an attribute set twice", "<ruleset a=\"1\" a=\"2\"/>", "attribute a set twice (line 1)"],
    ["an attribute without a value", "<r a/>", "attribute a has no value (line 1)"],
    ["an unquoted attribute value", "<r a=1/>", "an attribute value must be quoted (line 1)"],
    ["an unterminated attribute value", "<r a=\"1/>", "an unterminated attribute value (line 1)"],
    ["an attribute value holding <", "<r a=\"<\"/>", "an attribute value holds < (line 1)"],
    ["an element not closed", "<r>\n\n<s>text", "element s is not closed (line 3)"],
    ["an unterminated CDATA section", "<r><![CDATA[x</r>", "an unterminated CDATA section (line 1)"],
    ["a declaration in content", "<r><!ENTITY x></r>", "a declaration is not read (line 1)"],
    ["an end tag for another element", "<ruleset><rule></ruleset>", "</ruleset> closes <rule> (line 1)"],
    ["an end tag not closed", "<r></r x>", "</r> is not closed (line 1)"],
    ["an & that starts no reference", "<r>a & b</r>", "an & that is not a reference (line 1)"],
    ["another entity", "<ruleset>&a;</ruleset>", "the reference &a; is not read (line 1)"],
    ["a character reference past the last code point", "<r>&#x110000;</r>", "the reference &#x110000; is not read (line 1)"],
    ["a character reference without digits", "<r a=\"&#;\"/>", "the reference &#; is not read (line 1)"],
    ["an unbound prefix", "<x:ruleset/>", "the prefix of x:ruleset is not bound (line 1)"],
  ])("refuses %s", (_, text, why) => {
    expect(() => parseXml(text)).toThrow(LintConfigFormatError);
    expect(() => parseXml(text)).toThrow(`not XML Rerail reads: ${why}`);
  });
});

/** A NEON value as plain data: a list when no item has a key, else an object; an entity as `<entity>`. */
const plainNeon = (node: NeonValue): unknown => {
  if (typeof node !== "object" || node === null) return node;
  if (node.kind === "entity") return "<entity>";
  return node.items.every(item => item.key === undefined) ? node.items.map(item => plainNeon(item.value)) : Object.fromEntries(node.items.map(item => [item.key, plainNeon(item.value)]));
};

describe("parseNeon", () => {
  it("reads block and inline arrays, strings, comments and literals as nette/neon does", () => {
    const text = [
      "includes:",
      "\t- phpstan-baseline.neon # the baseline",
      "parameters:",
      "\tlevel: 6",
      "\tpaths: [src,",
      "\t\ttests]",
      "\tignoreErrors:",
      "\t\t- identifier: missingType.return",
      "\t\t  path: src/Legacy",
      "\t\t- '#^Call to an undefined method ''Foo''#'",
      "\t\t- message: \"tab\\there\"",
      "\t\t  reportUnmatched: false",
      "\ttreatPhpDocTypesAsCertain: no",
      "services:",
      "\t- Foo\\Bar(%currentWorkingDirectory%, [a: 1])",
      "multi: '''",
      "\tone",
      "\t  two",
      "'''",
    ].join("\r\n");
    expect(plainNeon(parseNeon(text))).toEqual({
      includes: ["phpstan-baseline.neon"],
      parameters: {
        level: 6,
        paths: ["src", "tests"],
        ignoreErrors: [
          { identifier: "missingType.return", path: "src/Legacy" },
          "#^Call to an undefined method 'Foo'#",
          { message: "tab\there", reportUnmatched: false },
        ],
        treatPhpDocTypesAsCertain: false,
      },
      services: ["<entity>"],
      multi: "one\n  two",
    });
  });

  it.each([
    ["an empty file as null", "", null],
    ["a file of comments as null", "# only\n\n# comments\n", null],
    ["a scalar file", "foo", "foo"],
    ["booleans, null, numbers and other literals", "[true, Yes, ON, false, no, Off, null, NULL, 1, -1.5, .5, 1e3, +1, 0x1F, 0o17, 0b101, 0x, 2024-01-02, foo bar]",
      [true, true, true, false, false, false, null, null, 1, -1.5, 0.5, 1000, 1, 31, 15, 5, "0x", "2024-01-02", "foo bar"]],
    ["literal keys as text, numbers as their value", "true: a\n1: b\n0x1F: c\n'q': d\nnull= e", { true: "a", 1: "b", 31: "c", q: "d", null: "e" }],
    ["every escape", String.raw`['a''b', "\t\n\r\f\b\"\\\/\_", "\u00E9\ud83d\ude00\x41"]`, ["a'b", "\t\n\r\f\b\"\\/\u00A0", "é😀A"]],
    ["a multi-line string, its indentation dropped and escapes read", "a: \"\"\"\n\tline\\tone\n\t  two\n\t\"\"\"", { a: "line\tone\n  two" }],
    ["a multi-line literal string as written", "a: '''\n  it''s\n  '''", { a: "it''s" }],
    ["an empty multi-line string", "'''\n'''", ""],
    ["a key's dash list at its own indentation", "a:\n- 1\nb: 2", { a: [1], b: 2 }],
    ["a key with nothing under it before a shallower key", "a:\n  b:\nc: 1", { a: { b: null }, c: 1 }],
    ["a key with nothing under it before a key beside it", "a:\nb: 1", { a: null, b: 1 }],
    ["a key with nothing after it at the end", "a:", { a: null }],
    ["a mapping after a dash", "- a: 1\n  b: 2\n- 3", [{ a: 1, b: 2 }, 3]],
    ["blank lines and comments between items, and =", "a=1\n\n\nb: 2  # c", { a: 1, b: 2 }],
    ["parentheses as an array", "a: (1, 2)", { a: [1, 2] }],
    ["inline keys without values", "{a: , b: 2, c:\n}", { a: null, b: 2, c: null }],
    ["inline items on separate lines", "[1\n2\n]", [1, 2]],
    ["a chain of entities", "a: Foo(1)Bar(2)baz\nb: Foo(1) bar", { a: "<entity>", b: "<entity>" }],
  ])("reads %s", (_, text, value) => {
    expect(plainNeon(parseNeon(text))).toEqual(value);
  });

  it.each([
    ["a key set twice", "a: 1\na: 2", "duplicated key 'a' on line 2"],
    ["a number key set twice as another literal", "1: a\n1.0: b", "duplicated key '1' on line 2"],
    ["an inline key set twice", "{a: 1, a: 2}", "duplicated key 'a' on line 1"],
    ["an array as a key", "[1]: 2", "unacceptable key on line 1"],
    ["a deeper line inside a block", "a:\n  b: 1\n    c: 2", "bad indentation on line 3"],
    ["tabs and spaces mixed between items", "a:\n\t b: 1\n  c: 2", "invalid combination of tabs and spaces on line 3"],
    ["tabs and spaces mixed under a key", "a:\n\tb:\n  c: 1", "invalid combination of tabs and spaces on line 3"],
    ["an unclosed inline array", "a: [1, 2", "unexpected end on line 1"],
    ["inline items without a separator", "[\"a\" \"b\"]", "unexpected '\"b\"' on line 1"],
    ["an unknown escape", "a: \"\\q\"", "invalid escaping sequence on line 1"],
    ["a short \\u escape", "a: \"\\u12\"", "invalid escaping sequence on line 1"],
    ["a short \\x escape", "a: \"\\x4\"", "invalid escaping sequence on line 1"],
    ["a key after a deeper first line", "  a: 1\nb: 2", "unexpected 'b' on line 2"],
    ["a value with no key after a mapping", "a: 1\nb", "unexpected end on line 2"],
    ["text after an inline value", "a: [1] x", "unexpected 'x' on line 1"],
    ["a comma for a value", "a: ,", "unexpected ',' on line 1"],
    ["a token over several lines, shown on one", "a: [1] '''\nxx\n'''", "unexpected ''''<new line>xx<new line>'''' on line 1"],
    ["a long token, cut at 40 characters", `a: [1] ${"x".repeat(50)}`, `unexpected '${"x".repeat(40)}' on line 1`],
    ["a character no token starts with, cut at 20", `a: \`${"y".repeat(30)}`, `unexpected '\`${"y".repeat(19)}' on line 1`],
    ["an unclosed string", "a: \"x", "unexpected '\"x' on line 1"],
  ])("refuses %s", (_, text, why) => {
    expect(() => parseNeon(text)).toThrow(LintConfigFormatError);
    expect(() => parseNeon(text)).toThrow(`not NEON: ${why}`);
  });

  it("finds an array's entry by key, and nothing in what is not an array", () => {
    const value = parseNeon("parameters:\n  level: 6\n");
    expect(isNeonArray(value)).toBe(true);
    expect(neonEntry(neonEntry(value, "parameters"), "level")).toBe(6);
    expect(neonEntry(value, "services")).toBeUndefined();
    expect(neonEntry(6, "level")).toBeUndefined();
    expect([null, undefined, "a", { kind: "entity" } as const].map(isNeonArray)).toEqual([false, false, false, false]);
  });
});
