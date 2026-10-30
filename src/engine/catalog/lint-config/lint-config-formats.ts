/**
 * The two small readers set-up needs for a repository's own lint config (`repository-lint-config.ts`): JSON with
 * comments and trailing commas, as ESLint's and oxlint's JSON config files allow, and the string lists Ruff reads
 * from TOML. Both refuse what they cannot read rather than guess: they throw `LintConfigFormatError`.
 */

export class LintConfigFormatError extends Error {}

/**
 * `text` as JSONC: JSON plus `//` and `/* *\/` comments and a trailing comma before `]` or `}`, a leading byte order
 * mark allowed. Throws `LintConfigFormatError` on anything else (YAML included).
 */
export function parseJsonc(text: string): unknown {
  const out: string[] = [];
  let pendingComma: number | undefined;
  let index = text.startsWith("\uFEFF") ? 1 : 0;
  while (index < text.length) {
    const char = text[index]!;
    const next = text[index + 1];
    if (char === "\"") {
      const end = jsonStringEnd(text, index);
      out.push(text.slice(index, end));
      pendingComma = undefined;
      index = end;
    } else if (char === "/" && next === "/") {
      const end = text.indexOf("\n", index);
      index = end < 0 ? text.length : end;
    } else if (char === "/" && next === "*") {
      const end = text.indexOf("*/", index + 2);
      if (end < 0) throw new LintConfigFormatError("a /* comment is not closed");
      out.push(" ");
      index = end + 2;
    } else {
      if ((char === "]" || char === "}") && pendingComma !== undefined) out[pendingComma] = " ";
      if (char === ",") pendingComma = out.length;
      else if (!/\s/u.test(char)) pendingComma = undefined;
      out.push(char);
      index += 1;
    }
  }
  try {
    return JSON.parse(out.join(""));
  } catch (error) {
    throw new LintConfigFormatError(`not JSON: ${(error as Error).message}`);
  }
}

/** The index just past the JSON string starting at `start` (a `"`). */
function jsonStringEnd(text: string, start: number): number {
  for (let index = start + 1; index < text.length; index += 1) {
    if (text[index] === "\\") index += 1;
    else if (text[index] === "\"") return index + 1;
    else if (text[index] === "\n") break;
  }
  throw new LintConfigFormatError("a string is not closed");
}

/** A TOML value as the reader keeps it: strings, arrays and inline tables; any other scalar only as present. */
type TomlValue = string | TomlValue[] | Map<string, TomlValue> | { readonly scalar: string };

/** What `readTomlStringLists` found: the wanted lists, tables and strings, and every key path the file sets. */
export interface TomlStringLists {
  /** Dotted key path (`tool.ruff.lint.ignore`) → its list, for each wanted path the file sets. */
  readonly lists: ReadonlyMap<string, readonly string[]>;
  /** Dotted table path (`tool.ruff.lint.per-file-ignores`) → each key it sets (unquoted) → its list, for each wanted table. */
  readonly tables: ReadonlyMap<string, ReadonlyMap<string, readonly string[]>>;
  /** Dotted key path (`tool.ruff.extend`) → its string, for each wanted string path the file sets. */
  readonly strings: ReadonlyMap<string, string>;
  /** Every key path the file sets, table headers included. */
  readonly paths: readonly string[];
}

const BARE_KEY = /[A-Za-z0-9_-]/u;
const BARE_KEY_TEXT = /^[A-Za-z0-9_-]+$/u;
/** A key path as TOML writes it: bare parts as they are, others quoted, so `"lint.ignore"` is not `lint.ignore`. */
const dottedPath = (parts: readonly string[]): string => parts.map(part => (BARE_KEY_TEXT.test(part) ? part : JSON.stringify(part))).join(".");
const ESCAPES: Readonly<Record<string, string>> = { b: "\b", t: "\t", n: "\n", f: "\f", r: "\r", "\"": "\"", "\\": "\\" };
/**
 * How many arrays and inline tables a value may nest in (a dotted key in an inline table nests a table for each part
 * before the last). The reader recurses into each, so a deeper file would overflow the stack.
 */
const MAX_NESTING = 64;

/**
 * A TOML reader for exactly what set-up reads: the string lists at `wanted` key paths, in the `wantedTables` tables
 * every key's string list, and the strings at `wantedStrings`. It parses the whole file's structure (tables, dotted and
 * quoted keys, strings of each kind, arrays over several lines with comments and trailing commas, inline tables) so
 * another key's value cannot be mistaken for one of them, and keeps other scalars unread. Throws
 * `LintConfigFormatError` (with the line) on what it cannot read, a wanted key of another type, or a key set twice.
 */
export function readTomlStringLists(text: string, wanted: readonly string[], wantedTables: readonly string[] = [], wantedStrings: readonly string[] = []): TomlStringLists {
  return new TomlReader(text, new Set(wanted), new Set(wantedTables), new Set(wantedStrings)).read();
}

class TomlReader {
  #index = 0;
  #table: string[] | undefined = [];
  readonly #lists = new Map<string, string[]>();
  readonly #tables = new Map<string, Map<string, string[]>>();
  readonly #strings = new Map<string, string>();
  readonly #paths = new Set<string>();

  constructor(
    readonly text: string,
    readonly wanted: ReadonlySet<string>,
    readonly wantedTables: ReadonlySet<string>,
    readonly wantedStrings: ReadonlySet<string>,
  ) {}

  read(): TomlStringLists {
    if (this.text.startsWith("\uFEFF")) this.#index = 1;
    for (;;) {
      this.#skipBlank(true);
      if (this.#index >= this.text.length) break;
      if (this.text[this.#index] === "[") this.#header();
      else this.#keyValue();
      this.#endOfLine();
    }
    return { lists: this.#lists, tables: this.#tables, strings: this.#strings, paths: [...this.#paths] };
  }

  #fail(problem: string): never {
    const line = this.text.slice(0, this.#index).split("\n").length;
    throw new LintConfigFormatError(`line ${line}: ${problem}`);
  }

  /** Spaces and tabs; with `newlines`, also line ends and comments. */
  #skipBlank(newlines: boolean): void {
    while (this.#index < this.text.length) {
      const char = this.text[this.#index]!;
      if (char === " " || char === "\t" || (newlines && (char === "\n" || char === "\r"))) this.#index += 1;
      else if (newlines && char === "#") this.#skipComment();
      else break;
    }
  }

  #skipComment(): void {
    const end = this.text.indexOf("\n", this.#index);
    this.#index = end < 0 ? this.text.length : end;
  }

  #endOfLine(): void {
    this.#skipBlank(false);
    if (this.text[this.#index] === "#") this.#skipComment();
    if (this.#index < this.text.length && this.text[this.#index] !== "\n" && this.text[this.#index] !== "\r") this.#fail("expected the end of the line");
  }

  #header(): void {
    const arrayTable = this.text.startsWith("[[", this.#index);
    this.#index += arrayTable ? 2 : 1;
    const path = this.#key();
    if (!this.text.startsWith(arrayTable ? "]]" : "]", this.#index)) this.#fail("a table header is not closed");
    this.#index += arrayTable ? 2 : 1;
    // No wanted key lives in an array of tables: its keys are read past, never collected.
    this.#table = arrayTable ? undefined : path;
    if (!arrayTable) {
      this.#paths.add(dottedPath(path));
      this.#wantedTable(dottedPath(path));
    }
  }

  /** A wanted table, recorded even while it is empty: an empty table is set, not absent. */
  #wantedTable(dotted: string): void {
    if (this.wantedTables.has(dotted) && !this.#tables.has(dotted)) this.#tables.set(dotted, new Map());
  }

  #keyValue(): void {
    const key = this.#key();
    if (this.text[this.#index] !== "=") this.#fail("expected =");
    this.#index += 1;
    this.#skipBlank(false);
    const value = this.#value(0);
    if (this.#table !== undefined) this.#collect([...this.#table, ...key], value);
  }

  /** Records `value` at `path`; `depth` is how many tables of the key's value enclose it. */
  #collect(path: readonly string[], value: TomlValue, depth = 0): void {
    const dotted = dottedPath(path);
    if (this.#paths.has(dotted) && !(value instanceof Map)) this.#fail(`${dotted} is set twice`);
    this.#paths.add(dotted);
    if (value instanceof Map) {
      if (depth >= MAX_NESTING) this.#fail(`values nest more than ${MAX_NESTING} deep`);
      this.#wantedTable(dotted);
      for (const [key, inner] of value) this.#collect([...path, key], inner, depth + 1);
      return;
    }
    if (this.wantedTables.has(dotted)) this.#fail(`${dotted} is not a table`);
    if (this.wantedStrings.has(dotted)) {
      if (typeof value !== "string") this.#fail(`${dotted} is not a string`);
      this.#strings.set(dotted, value);
      return;
    }
    const table = dottedPath(path.slice(0, -1));
    if (!this.wanted.has(dotted) && !this.wantedTables.has(table)) return;
    if (!Array.isArray(value) || !value.every(item => typeof item === "string")) this.#fail(`${dotted} is not a list of strings`);
    if (this.wanted.has(dotted)) this.#lists.set(dotted, value as string[]);
    else this.#tables.set(table, (this.#tables.get(table) ?? new Map<string, string[]>()).set(path.at(-1)!, value as string[]));
  }

  /** A dotted key of bare and quoted parts; leaves the index after the blanks that follow it. */
  #key(): string[] {
    const parts: string[] = [];
    for (;;) {
      this.#skipBlank(false);
      const char = this.text[this.#index];
      if (char === "\"" || char === "'") {
        if (this.text.startsWith(char.repeat(3), this.#index)) this.#fail("a key cannot be a multi-line string");
        parts.push(this.#string());
      } else {
        const start = this.#index;
        while (this.#index < this.text.length && BARE_KEY.test(this.text[this.#index]!)) this.#index += 1;
        if (this.#index === start) this.#fail("expected a key");
        parts.push(this.text.slice(start, this.#index));
      }
      this.#skipBlank(false);
      if (this.text[this.#index] !== ".") return parts;
      this.#index += 1;
    }
  }

  /** A value inside `depth` arrays and inline tables. */
  #value(depth: number): TomlValue {
    const char = this.text[this.#index];
    if (char === "\"" || char === "'") return this.#string();
    if (char === "[" || char === "{") {
      if (depth >= MAX_NESTING) this.#fail(`values nest more than ${MAX_NESTING} deep`);
      return char === "[" ? this.#array(depth + 1) : this.#inlineTable(depth + 1);
    }
    // A number, boolean or date: never wanted, so only its extent matters.
    const start = this.#index;
    while (this.#index < this.text.length && !"\n\r#,]}".includes(this.text[this.#index]!)) this.#index += 1;
    const scalar = this.text.slice(start, this.#index).trim();
    if (!/^[A-Za-z0-9_:.+\- ]+$/u.test(scalar)) this.#fail("expected a value");
    this.#index = start + this.text.slice(start, this.#index).trimEnd().length;
    return { scalar };
  }

  #array(depth: number): TomlValue[] {
    this.#index += 1;
    const items: TomlValue[] = [];
    for (;;) {
      this.#skipBlank(true);
      if (this.text[this.#index] === "]") break;
      items.push(this.#value(depth));
      this.#skipBlank(true);
      if (this.text[this.#index] === ",") this.#index += 1;
      else if (this.text[this.#index] !== "]") this.#fail("expected , or ] in a list");
    }
    this.#index += 1;
    return items;
  }

  #inlineTable(depth: number): Map<string, TomlValue> {
    this.#index += 1;
    const table = new Map<string, TomlValue>();
    this.#skipBlank(false);
    if (this.text[this.#index] === "}") {
      this.#index += 1;
      return table;
    }
    for (;;) {
      const key = this.#key();
      if (this.text[this.#index] !== "=") this.#fail("expected =");
      this.#index += 1;
      this.#skipBlank(false);
      let target = table;
      for (const part of key.slice(0, -1)) {
        const inner = target.get(part) ?? new Map<string, TomlValue>();
        if (!(inner instanceof Map)) this.#fail(`${key.join(".")} is set twice`);
        target.set(part, inner);
        target = inner;
      }
      const last = key.at(-1)!;
      if (target.has(last)) this.#fail(`${key.join(".")} is set twice`);
      target.set(last, this.#value(depth));
      this.#skipBlank(false);
      const char = this.text[this.#index];
      this.#index += 1;
      if (char === "}") return table;
      if (char !== ",") this.#fail("expected , or } in an inline table");
    }
  }

  #string(): string {
    const quote = this.text[this.#index]!;
    const multiLine = this.text.startsWith(quote.repeat(3), this.#index);
    this.#index += multiLine ? 3 : 1;
    // A multi-line string's first line break is not part of it.
    if (multiLine && this.text.startsWith("\r\n", this.#index)) this.#index += 2;
    else if (multiLine && this.text[this.#index] === "\n") this.#index += 1;
    let value = "";
    for (;;) {
      if (this.#index >= this.text.length) this.#fail("a string is not closed");
      const char = this.text[this.#index]!;
      if (multiLine && this.text.startsWith(quote.repeat(3), this.#index)) {
        // Up to two quotes may end the content just before the closing three.
        let extra = 0;
        while (extra < 2 && this.text[this.#index + 3 + extra] === quote) extra += 1;
        value += quote.repeat(extra);
        this.#index += 3 + extra;
        return value;
      }
      if (!multiLine && char === quote) {
        this.#index += 1;
        return value;
      }
      if (!multiLine && (char === "\n" || char === "\r")) this.#fail("a string is not closed");
      if (quote === "\"" && char === "\\") {
        value += this.#escape(multiLine);
        continue;
      }
      value += char;
      this.#index += 1;
    }
  }

  #escape(multiLine: boolean): string {
    const code = this.text[this.#index + 1] ?? "";
    if (code in ESCAPES) {
      this.#index += 2;
      return ESCAPES[code]!;
    }
    if (code === "u" || code === "U") {
      const digits = this.text.slice(this.#index + 2, this.#index + (code === "u" ? 6 : 10));
      if (!/^[0-9A-Fa-f]+$/u.test(digits) || digits.length !== (code === "u" ? 4 : 8)) this.#fail("a \\u escape needs hex digits");
      const point = Number.parseInt(digits, 16);
      if (point > 0x10ffff || (point >= 0xd800 && point <= 0xdfff)) this.#fail("a \\u escape is not a Unicode scalar value");
      this.#index += code === "u" ? 6 : 10;
      return String.fromCodePoint(point);
    }
    // A line-ending backslash in a multi-line basic string trims the line break and the blanks after it.
    const rest = /^\\[ \t]*\r?\n[\s]*/u.exec(this.text.slice(this.#index));
    if (multiLine && rest !== null) {
      this.#index += rest[0].length;
      return "";
    }
    this.#fail(`unknown escape \\${code}`);
  }
}
