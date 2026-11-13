import { describe, expect, it } from "vitest";
import { LintConfigFormatError, readTomlStringLists, readTomlTrees, type TomlValue } from "../../../../src/engine/catalog/lint-config/formats/lint-config-formats.js";

/** A tree as plain objects, for comparing. */
const plain = (value: TomlValue | ReadonlyMap<string, TomlValue> | undefined): unknown => {
  if (value instanceof Map) return Object.fromEntries([...value].map(([key, inner]) => [key, plain(inner)]));
  return Array.isArray(value) ? value.map(plain) : value;
};

/** The string at key `key` of a file that is only `key = <value>`. */
const stringOf = (value: string): string | undefined => readTomlStringLists(`key = ${value}`, [], [], ["key"]).strings.get("key");

const PYPROJECT_KEYS = ["tool.ruff.ignore", "tool.ruff.extend-ignore", "tool.ruff.lint.ignore", "tool.ruff.lint.extend-ignore"];

describe("readTomlStringLists lists", () => {
  it("reads the wanted lists across lines, with comments, both quotes and a trailing comma", () => {
    const text = [
      "[project]",
      "name = \"acme\" # a comment",
      "description = \"\"\"",
      "[tool.ruff.lint]",
      "ignore = [\"X1\"]",
      "\"\"\"",
      "version = 1.2",
      "released = 1979-05-27 07:32:00Z",
      "",
      "[tool.ruff]",
      "line-length = 100",
      "lint.extend-ignore = ['D1']",
      "",
      "[ tool . ruff . \"lint\" ]",
      "ignore = [",
      "  \"E7\",   # comparisons",
      "",
      "  'F401',",
      "]",
      "select = [\"E\", \"F\"]",
      "",
      "[[tool.other]]",
      "ignore = [1]",
    ].join("\n");
    const found = readTomlStringLists(text, PYPROJECT_KEYS);
    expect(Object.fromEntries(found.lists)).toEqual({ "tool.ruff.lint.extend-ignore": ["D1"], "tool.ruff.lint.ignore": ["E7", "F401"] });
    expect(found.paths).toEqual([
      "project", "project.name", "project.description", "project.version", "project.released",
      "tool.ruff", "tool.ruff.line-length", "tool.ruff.lint.extend-ignore", "tool.ruff.lint", "tool.ruff.lint.ignore", "tool.ruff.lint.select",
    ]);
  });

  it("reads a byte order mark, CRLF line ends and tabs", () => {
    const found = readTomlStringLists("\uFEFF[tool.ruff.lint]\r\n\tignore\t=\t[\"E1\",\r\n\"E2\"]\t# two\r\n", PYPROJECT_KEYS);
    expect(Object.fromEntries(found.lists)).toEqual({ "tool.ruff.lint.ignore": ["E1", "E2"] });
  });

  it("reads inline tables, and keeps a quoted dotted key apart", () => {
    const found = readTomlStringLists("[tool]\nruff = { lint = { ignore = [\"E1\"] }, \"lint.extend-ignore\" = [\"E2\"] }\n", PYPROJECT_KEYS);
    expect(Object.fromEntries(found.lists)).toEqual({ "tool.ruff.lint.ignore": ["E1"] });
    expect(found.paths).toContain("tool.ruff.\"lint.extend-ignore\"");
  });

  it("reads an empty inline table, an empty list and dotted keys inside an inline table", () => {
    const found = readTomlStringLists("a = {}\nb = { }\nc = []\nd = { x.y = 1, x.z = ['s'] }\n", ["d.x.z"]);
    expect(Object.fromEntries(found.lists)).toEqual({ "d.x.z": ["s"] });
    expect(found.paths).toEqual(["a", "b", "c", "d", "d.x", "d.x.y", "d.x.z"]);
  });

  it("collects nothing in an array of tables, and reads on after it", () => {
    const found = readTomlStringLists("[[tool.ruff.lint]]\nignore = [\"E1\"]\n[[tool.ruff.lint]]\nignore = [\"E2\"]\n[tool.ruff]\nignore = [\"E3\"]\n", PYPROJECT_KEYS);
    expect(Object.fromEntries(found.lists)).toEqual({ "tool.ruff.ignore": ["E3"] });
    expect(found.paths).toEqual(["tool.ruff", "tool.ruff.ignore"]);
  });

  it("lets a table be added to, by a header after an inline table at its path", () => {
    const found = readTomlStringLists("[a]\nb = { c = [\"x\"] }\n[a.b]\nd = [\"y\"]\n", ["a.b.c", "a.b.d"]);
    expect(Object.fromEntries(found.lists)).toEqual({ "a.b.c": ["x"], "a.b.d": ["y"] });
  });
});

describe("readTomlStringLists tables, strings and scalars", () => {
  const TABLE = "tool.ruff.lint.per-file-ignores";

  it("reads every key's list in a wanted table, however the table is written", () => {
    const text = [
      "[tool.ruff.lint.per-file-ignores]",
      "\"tests/*.py\" = [\"S101\"]",
      "[tool.ruff.lint]",
      "per-file-ignores.\"a.py\" = [\"E1\"]",
      "other = { b = [1] }",
    ].join("\n");
    const found = readTomlStringLists(text, [], [TABLE]);
    expect(Object.fromEntries([...found.tables].map(([table, keys]) => [table, Object.fromEntries(keys)]))).toEqual({ [TABLE]: { "tests/*.py": ["S101"], "a.py": ["E1"] } });
  });

  it("records a wanted table that is empty, keeps the parts of one set twice, and leaves out one the file does not set", () => {
    expect([...readTomlStringLists(`[${TABLE}]\n`, [], [TABLE, "other"]).tables]).toEqual([[TABLE, new Map()]]);
    expect([...readTomlStringLists("t = { }\n", [], ["t"]).tables]).toEqual([["t", new Map()]]);
    const twice = readTomlStringLists("t.k = [\"x\"]\n[t]\nj = [\"y\"]\n", [], ["t"]);
    expect(Object.fromEntries(twice.tables.get("t")!)).toEqual({ k: ["x"], j: ["y"] });
  });

  it("reads a wanted string and a wanted scalar's text", () => {
    const found = readTomlStringLists("[tool.ruff]\nextend = \"../base.toml\"\npreview = true # on\nline-length = 88\n", [], [], ["tool.ruff.extend"], ["tool.ruff.preview", "tool.ruff.line-length"]);
    expect(Object.fromEntries(found.strings)).toEqual({ "tool.ruff.extend": "../base.toml" });
    expect(Object.fromEntries(found.scalars)).toEqual({ "tool.ruff.preview": "true", "tool.ruff.line-length": "88" });
  });

  it.each([
    ["a list that is a string", "[tool.ruff.lint]\nignore = \"E7\"\n", "line 2: tool.ruff.lint.ignore is not a list of strings"],
    ["a list of numbers", "[tool.ruff.lint]\nignore = [7]\n", "line 2: tool.ruff.lint.ignore is not a list of strings"],
    ["a wanted table's key that is not a list", `[${TABLE}]\n"a.py" = "E1"\n`, `line 2: ${TABLE}."a.py" is not a list of strings`],
    ["a wanted table set as a value", "[tool.ruff.lint]\nper-file-ignores = []\n", `line 2: ${TABLE} is not a table`],
    ["a wanted string that is a list", "[tool.ruff]\nextend = [\"a\"]\n", "line 2: tool.ruff.extend is not a string"],
    ["a wanted scalar that is a string", "[tool.ruff]\npreview = \"true\"\n", "line 2: tool.ruff.preview is not a scalar"],
    ["a wanted scalar that is a list", "[tool.ruff]\npreview = [true]\n", "line 2: tool.ruff.preview is not a scalar"],
    ["a wanted list set by a table header", "[project]\n[tool.ruff.lint.ignore]\n", "line 2: tool.ruff.lint.ignore is not a list of strings"],
    ["a wanted list set by an array-of-tables header", "[[tool.ruff.lint.ignore]]\n", "line 1: tool.ruff.lint.ignore is not a list of strings"],
    ["a wanted list under a table header", "[tool.ruff.lint.ignore.E7]\n", "line 1: tool.ruff.lint.ignore is not a list of strings"],
    ["a wanted list set by a dotted key", "[tool.ruff.lint]\nignore.E7 = true\n", "line 2: tool.ruff.lint.ignore is not a list of strings"],
    ["a wanted list set as an inline table", "[tool.ruff.lint]\nignore = { E7 = true }\n", "line 2: tool.ruff.lint.ignore is not a list of strings"],
    ["a wanted list set in a nested inline table", "[tool]\nruff = { ignore = {} }\n", "line 2: tool.ruff.ignore is not a list of strings"],
    ["a wanted string set by a dotted key", "[tool.ruff]\nextend.path = \"a\"\n", "line 2: tool.ruff.extend is not a string"],
    ["a wanted scalar set by a table header", "[tool.ruff.preview]\n", "line 1: tool.ruff.preview is not a scalar"],
    ["a wanted scalar set as an empty inline table", "[tool.ruff]\npreview = {}\n", "line 2: tool.ruff.preview is not a scalar"],
  ])("refuses %s", (_, text, message) => {
    expect(() => readTomlStringLists(text, PYPROJECT_KEYS, [TABLE], ["tool.ruff.extend"], ["tool.ruff.preview"])).toThrow(message);
  });
});

describe("readTomlStringLists strings", () => {
  it.each([
    ["every basic escape", String.raw`"\b\t\n\f\r\"\\"`, "\b\t\n\f\r\"\\"],
    ["a \\u and a \\U escape", String.raw`"\u00e9\U0001F600\uD7FF\uE000"`, "é😀\uD7FF\uE000"],
    ["a literal string, backslashes as written", String.raw`'C:\q\n'`, String.raw`C:\q\n`],
    ["a multi-line basic string, its first line break dropped", "\"\"\"\nline one\n\"line\" two\"\"\"", "line one\n\"line\" two"],
    ["a multi-line string after a CRLF", "\"\"\"\r\nx\"\"\"", "x"],
    ["a multi-line string on its opening line", "'''x\ny'''", "x\ny"],
    ["up to two quotes before the closing three", "\"\"\"x\"\"\"\"\"", "x\"\""],
    ["one quote before the closing three", "'''x''''", "x'"],
    ["a line-ending backslash trimming the break and blanks after it", "\"\"\"a \\  \n  \n   b \\\r\n c\"\"\"", "a b c"],
    ["a multi-line literal string, backslashes as written", "'''a \\\nb'''", "a \\\nb"],
  ])("reads %s", (_, value, text) => {
    expect(stringOf(value)).toBe(text);
  });

  it.each([
    ["an unknown escape", String.raw`"\q"`, "line 1: unknown escape \\q"],
    ["a line-ending backslash in a one-line string", "\"a\\\nb\"", "line 1: unknown escape \\\n"],
    ["a backslash at the end of the file", "\"a\\", "line 1: unknown escape \\"],
    ["a \\u escape with a letter that is not hex", String.raw`"\u12G4"`, "line 1: a \\u escape needs hex digits"],
    ["a \\u escape cut short by the end of the file", "\"\\u1", "line 1: a \\u escape needs hex digits"],
    ["a \\u escape that is a surrogate", String.raw`"\uD800"`, "line 1: a \\u escape is not a Unicode scalar value"],
    ["a \\U escape past the last code point", String.raw`"\U00110000"`, "line 1: a \\u escape is not a Unicode scalar value"],
    ["a string open at the end of the file", "\"open", "line 1: a string is not closed"],
    ["a string open at the end of its line", "'open\nkey2 = 1", "line 1: a string is not closed"],
    ["a string open at a carriage return", "\"open\r\n", "line 1: a string is not closed"],
    ["a multi-line string open at the end of the file", "\"\"\"\nopen\n", "line 3: a string is not closed"],
  ])("refuses %s", (_, value, message) => {
    expect(() => stringOf(value)).toThrow(LintConfigFormatError);
    expect(() => stringOf(value)).toThrow(message);
  });
});

describe("readTomlStringLists refusals", () => {
  it.each([
    ["a list not closed", "[tool.ruff.lint]\nignore = [\"E7\"\n", "line 3: expected , or ] in a list"],
    ["list items without a comma", "a = [\"x\" \"y\"]\n", "line 1: expected , or ] in a list"],
    ["a key set twice across tables", "[tool.ruff]\nlint.ignore = []\n[tool.ruff.lint]\nignore = []\n", "line 4: tool.ruff.lint.ignore is set twice"],
    ["text after a value", "[tool.ruff.lint]\nignore = [\"E7\"] extra\n", "line 2: expected the end of the line"],
    ["a table header not closed", "[tool.ruff\n", "line 1: a table header is not closed"],
    ["an array-of-tables header not closed", "[[tool.ruff]\n", "line 1: a table header is not closed"],
    ["a key without =", "\na b\n", "line 2: expected ="],
    ["a key at the end of the file", "a", "line 1: expected ="],
    ["an empty key", "= 1\n", "line 1: expected a key"],
    ["a multi-line basic string as a key", "\"\"\"a\"\"\" = 1\n", "line 1: a key cannot be a multi-line string"],
    ["a multi-line literal string as a key", "'''a''' = 1\n", "line 1: a key cannot be a multi-line string"],
    ["a value that is no value", "a = @\n", "line 1: expected a value"],
    ["no value at all", "a =\n", "line 1: expected a value"],
    ["an inline table key without =", "a = { b }\n", "line 1: expected ="],
    ["an inline table key set twice", "a = { b = 1, b = 2 }\n", "line 1: b is set twice"],
    ["an inline table key under a value", "a = { b = 1, b.c = 2 }\n", "line 1: b.c is set twice"],
    ["inline table keys without a comma", "a = { b = \"1\" c = 2 }\n", "line 1: expected , or } in an inline table"],
  ])("refuses %s", (_, text, message) => {
    expect(() => readTomlStringLists(text, PYPROJECT_KEYS)).toThrow(LintConfigFormatError);
    expect(() => readTomlStringLists(text, PYPROJECT_KEYS)).toThrow(message);
  });

  it("reads a scalar's extent up to a comment, a comma or a closing bracket, its trailing blanks dropped", () => {
    const found = readTomlStringLists("a = 1979-05-27 07:32:00   # when\nb = [1 , +2.5e3]\nc = { d = -inf } # no line end after", [], [], [], ["a", "c.d"]);
    expect(Object.fromEntries(found.scalars)).toEqual({ a: "1979-05-27 07:32:00", "c.d": "-inf" });
  });

  // PR #50 review: values nested without end overflowed the stack, an error set-up does not catch.
  it("refuses values nested more than 64 deep, and reads shallower ones", () => {
    expect(readTomlStringLists("x = [[[\"a\"]]]\ny = { a = { b = { c = 1 } } }\n", []).paths).toEqual(["x", "y", "y.a", "y.a.b", "y.a.b.c"]);
    expect(readTomlStringLists(`x = ${"[".repeat(64)}${"]".repeat(64)}\n`, []).paths).toEqual(["x"]);
    expect(readTomlStringLists(`x = ${"{ a = ".repeat(63)}1${" }".repeat(63)}\n`, []).paths).toHaveLength(64);
    expect(() => readTomlStringLists(`x = ${"[".repeat(65)}${"]".repeat(65)}\n`, [])).toThrow("line 1: values nest more than 64 deep");
    expect(() => readTomlStringLists(`x = ${"{ a = ".repeat(65)}1${" }".repeat(65)}\n`, [])).toThrow("line 1: values nest more than 64 deep");
    // A dotted key in an inline table nests a table for each part before the last.
    expect(readTomlStringLists(`x = { ${"a.".repeat(63)}a = 1 }\n`, []).paths).toHaveLength(65);
    expect(() => readTomlStringLists(`x = { ${"a.".repeat(64)}a = 1 }\n`, [])).toThrow("line 1: values nest more than 64 deep");
    expect(() => readTomlStringLists(`x = ${"[".repeat(100_000)}\n`, [])).toThrow("line 1: values nest more than 64 deep");
    expect(() => readTomlStringLists(`x = { ${"a.".repeat(100_000)}a = 1 }\n`, [])).toThrow("line 1: values nest more than 64 deep");
  });

  // A dotted key of 100,000 parts looked for a wanted tree at each of its prefixes, rebuilding each: minutes of work.
  it("reads a dotted key of very many parts in one pass", () => {
    const key = `${"a.".repeat(100_000)}a`;
    expect(readTomlTrees(`${key} = 1\n`, ["lints"])).toEqual({ trees: new Map(), paths: [key] });
    expect(readTomlStringLists(`${key} = 1\n`, ["a.a.b"], [], ["b"], ["a.c"]).paths).toEqual([key]);
    expect(() => readTomlStringLists(`${key} = 1\n`, ["a.a.a"])).toThrow("line 1: a.a.a is not a list of strings");
  });
});

describe("readTomlTrees", () => {
  it("reads a wanted table whole, however the file writes it", () => {
    const text = [
      "[package]",
      "name = \"demo\"",
      "[lints]",
      "workspace = true",
      "rust.warnings = \"deny\"",
      "[lints.clippy]",
      "all = { level = \"warn\", priority = -1 }",
      "[lints.clippy.pedantic]",
      "level = 'allow'",
      "[workspace.lints.clippy]",
      "style = \"allow\"",
    ].join("\n");
    const { trees, paths } = readTomlTrees(text, ["lints", "workspace.lints"]);
    expect(plain(trees.get("lints"))).toEqual({
      workspace: { scalar: "true" },
      rust: { warnings: "deny" },
      clippy: { all: { level: "warn", priority: { scalar: "-1" } }, pedantic: { level: "allow" } },
    });
    expect(plain(trees.get("workspace.lints"))).toEqual({ clippy: { style: "allow" } });
    expect(paths).toContain("package.name");
  });

  it("reads a tree set by dotted keys before its header, and a header after its sub-tables", () => {
    expect(plain(readTomlTrees("lints.clippy.all = \"warn\"\n", ["lints"]).trees.get("lints"))).toEqual({ clippy: { all: "warn" } });
    const later = readTomlTrees("[lints.clippy]\nall = \"warn\"\n[lints]\nrust = { unsafe = \"deny\" }\n", ["lints"]);
    expect(plain(later.trees.get("lints"))).toEqual({ clippy: { all: "warn" }, rust: { unsafe: "deny" } });
  });

  it("records a wanted table that is empty, and leaves out one the file does not set", () => {
    const { trees } = readTomlTrees("[lints]\n[package]\nlints = 1\n", ["lints", "workspace.lints"]);
    expect([...trees.keys()]).toEqual(["lints"]);
    expect(trees.get("lints")?.size).toBe(0);
  });

  it("refuses a key set twice in a wanted table, or a table under a value", () => {
    expect(() => readTomlTrees("[lints.clippy]\nall = \"warn\"\nall = \"deny\"\n", ["lints"])).toThrow("line 3: lints.clippy.all is set twice");
    expect(() => readTomlTrees("[lints]\nclippy = \"warn\"\n[lints.clippy]\nall = \"deny\"\n", ["lints"])).toThrow("line 4: lints.clippy.all is set under a value that is not a table");
  });
});
