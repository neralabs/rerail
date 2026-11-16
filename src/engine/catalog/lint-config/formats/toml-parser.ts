/**
 * TOML's syntax, parsed for the TOML reader (`toml-format.ts`): table headers, dotted keys of bare and quoted parts,
 * strings of each kind with their escapes, arrays over several lines with comments and trailing commas, and inline
 * tables. It parses every value, wanted or not, so another key's value cannot be mistaken for a wanted one, and keeps
 * numbers, booleans and dates as their text, unread. Which keys are wanted is the reader's business, not the parser's.
 * On what it cannot read it throws `LintConfigFormatError` with the line.
 */
import { LintConfigFormatError } from "./lint-config-formats.js";
import type { TomlValue, TomlVersion } from "./toml-format.js";

const BARE_KEY = /[A-Za-z0-9_-]/u;
const ESCAPES: Readonly<Record<string, string>> = { b: "\b", t: "\t", n: "\n", f: "\f", r: "\r", "\"": "\"", "\\": "\\" };
/** The hex digits of each `\u`-style escape. */
const HEX_ESCAPES: Readonly<Record<string, number>> = { u: 4, U: 8 };
/** What `#skipBlank` skips on a line, and what it skips across lines. */
const BLANKS = " \t";
const LINE_BLANKS = " \t\n\r";

/**
 * How many arrays and inline tables a value may nest in (a dotted key in an inline table nests a table for each part
 * before the last). The reader recurses into each, so a deeper file would overflow the stack.
 */
export const MAX_NESTING = 64;

/** A table header: its key path, and whether it is an array of tables (`[[path]]`) rather than a table (`[path]`). */
export interface TomlHeader {
  readonly path: string[];
  readonly arrayTable: boolean;
}

export class TomlParser {
  readonly #text: string;
  #index: number;
  readonly #escapes: Readonly<Record<string, string>>;
  readonly #hexEscapes: Readonly<Record<string, number>>;
  /** Whether an inline table may run over several lines (TOML 1.1). */
  readonly #multiLineInlineTables: boolean;

  constructor(text: string, version: TomlVersion) {
    const toml11 = version === "1.1";
    this.#text = text;
    this.#index = text.startsWith("\uFEFF") ? 1 : 0;
    this.#escapes = toml11 ? { ...ESCAPES, e: "\u001b" } : ESCAPES;
    this.#hexEscapes = toml11 ? { ...HEX_ESCAPES, x: 2 } : HEX_ESCAPES;
    this.#multiLineInlineTables = toml11;
  }

  /** Throws `LintConfigFormatError` for `problem`, naming the line the parser is on. */
  fail(problem: string): never {
    const line = this.#text.slice(0, this.#index).split("\n").length;
    throw new LintConfigFormatError(`line ${line}: ${problem}`);
  }

  /** Skips blank lines and comments; whether a line is left to read. */
  nextLine(): boolean {
    this.#skipBlank(true);
    return this.#index < this.#text.length;
  }

  atTableHeader(): boolean {
    return this.#text[this.#index] === "[";
  }

  tableHeader(): TomlHeader {
    const arrayTable = this.#text.startsWith("[[", this.#index);
    const close = arrayTable ? "]]" : "]";
    this.#index += close.length;
    const path = this.#key();
    if (!this.#text.startsWith(close, this.#index)) this.fail("a table header is not closed");
    this.#index += close.length;
    return { path, arrayTable };
  }

  /** A `key = value` line's key and value. */
  keyValue(): { readonly key: string[]; readonly value: TomlValue } {
    const key = this.#key();
    this.#equals();
    return { key, value: this.#value(0) };
  }

  /**
   * One `key = value` of an inline table or of a wanted array's element, a dotted key nesting a table for each part
   * before the last.
   */
  inlineEntry(table: Map<string, TomlValue>, depth: number): void {
    const key = this.#key();
    this.#equals();
    let target = table;
    for (const part of key.slice(0, -1)) {
      const inner = target.get(part) ?? new Map<string, TomlValue>();
      if (!(inner instanceof Map)) this.fail(`${key.join(".")} is set twice`);
      target.set(part, inner);
      target = inner;
    }
    const last = key.at(-1)!;
    if (target.has(last)) this.fail(`${key.join(".")} is set twice`);
    target.set(last, this.#value(depth));
  }

  endOfLine(): void {
    this.#skipBlank(false);
    if (this.#text[this.#index] === "#") this.#skipComment();
    if (this.#index < this.#text.length && this.#text[this.#index] !== "\n" && this.#text[this.#index] !== "\r") this.fail("expected the end of the line");
  }

  /** Spaces and tabs; with `newlines`, also line ends and comments. */
  #skipBlank(newlines: boolean): void {
    const blanks = newlines ? LINE_BLANKS : BLANKS;
    while (this.#index < this.#text.length) {
      const char = this.#text[this.#index]!;
      if (blanks.includes(char)) this.#index += 1;
      else if (newlines && char === "#") this.#skipComment();
      else break;
    }
  }

  #skipComment(): void {
    const end = this.#text.indexOf("\n", this.#index);
    this.#index = end < 0 ? this.#text.length : end;
  }

  /** The `=` after a key, and the blanks after it. */
  #equals(): void {
    if (this.#text[this.#index] !== "=") this.fail("expected =");
    this.#index += 1;
    this.#skipBlank(false);
  }

  /** A dotted key of bare and quoted parts; leaves the index after the blanks that follow it. */
  #key(): string[] {
    const parts: string[] = [];
    for (;;) {
      this.#skipBlank(false);
      parts.push(this.#keyPart());
      this.#skipBlank(false);
      if (this.#text[this.#index] !== ".") return parts;
      this.#index += 1;
    }
  }

  /** One part of a dotted key: a quoted string, never a multi-line one, or a bare key. */
  #keyPart(): string {
    const char = this.#text[this.#index];
    if (char !== "\"" && char !== "'") return this.#bareKey();
    if (this.#text.startsWith(char.repeat(3), this.#index)) this.fail("a key cannot be a multi-line string");
    return this.#string();
  }

  #bareKey(): string {
    const start = this.#index;
    while (this.#index < this.#text.length && BARE_KEY.test(this.#text[this.#index]!)) this.#index += 1;
    if (this.#index === start) this.fail("expected a key");
    return this.#text.slice(start, this.#index);
  }

  /** A value inside `depth` arrays and inline tables. */
  #value(depth: number): TomlValue {
    const char = this.#text[this.#index];
    if (char === "\"" || char === "'") return this.#string();
    if (char === "[" || char === "{") return this.#nested(char, depth);
    return this.#scalar();
  }

  /** The array (`[`) or inline table (`{`) that opens inside `depth` others. */
  #nested(open: string, depth: number): TomlValue {
    if (depth >= MAX_NESTING) this.fail(`values nest more than ${MAX_NESTING} deep`);
    return open === "[" ? this.#array(depth + 1) : this.#inlineTable(depth + 1);
  }

  /** A number, boolean or date: never wanted, so only its extent matters. */
  #scalar(): TomlValue {
    const start = this.#index;
    while (this.#index < this.#text.length && !"\n\r#,]}".includes(this.#text[this.#index]!)) this.#index += 1;
    const scalar = this.#text.slice(start, this.#index).trim();
    if (!/^[A-Za-z0-9_:.+\- ]+$/u.test(scalar)) this.fail("expected a value");
    this.#index = start + this.#text.slice(start, this.#index).trimEnd().length;
    return { scalar };
  }

  #array(depth: number): TomlValue[] {
    this.#index += 1;
    const items: TomlValue[] = [];
    for (;;) {
      this.#skipBlank(true);
      if (this.#text[this.#index] === "]") break;
      items.push(this.#value(depth));
      this.#skipBlank(true);
      if (this.#text[this.#index] === ",") this.#index += 1;
      else if (this.#text[this.#index] !== "]") this.fail("expected , or ] in a list");
    }
    this.#index += 1;
    return items;
  }

  #inlineTable(depth: number): Map<string, TomlValue> {
    this.#index += 1;
    const table = new Map<string, TomlValue>();
    const lines = this.#multiLineInlineTables;
    for (;;) {
      this.#skipBlank(lines);
      if (this.#closesEarly(table)) return table;
      this.inlineEntry(table, depth);
      this.#skipBlank(lines);
      const char = this.#text[this.#index];
      this.#index += 1;
      if (char === "}") return table;
      if (char !== ",") this.fail("expected , or } in an inline table");
    }
  }

  /** Whether a `}` closes the inline table before an entry: `{}`, or in TOML 1.1 the `}` after a trailing comma. */
  #closesEarly(table: ReadonlyMap<string, TomlValue>): boolean {
    if (this.#text[this.#index] !== "}" || (table.size > 0 && !this.#multiLineInlineTables)) return false;
    this.#index += 1;
    return true;
  }

  #string(): string {
    const quote = this.#text[this.#index]!;
    const multiLine = this.#text.startsWith(quote.repeat(3), this.#index);
    this.#index += multiLine ? 3 : 1;
    if (multiLine) this.#skipFirstLineBreak();
    let value = "";
    for (;;) {
      const closing = this.#closing(quote, multiLine);
      if (closing !== undefined) return value + closing;
      value += this.#stringChar(quote, multiLine);
    }
  }

  /** A multi-line string's first line break is not part of it. */
  #skipFirstLineBreak(): void {
    if (this.#text.startsWith("\r\n", this.#index)) this.#index += 2;
    else if (this.#text[this.#index] === "\n") this.#index += 1;
  }

  /**
   * At the string's closing quotes: what of them ends its content, the index moved past them; else undefined. Fails
   * where the string runs out unclosed.
   */
  #closing(quote: string, multiLine: boolean): string | undefined {
    if (this.#index >= this.#text.length) this.fail("a string is not closed");
    return multiLine ? this.#multiLineClosing(quote) : this.#singleLineClosing(quote);
  }

  #multiLineClosing(quote: string): string | undefined {
    if (!this.#text.startsWith(quote.repeat(3), this.#index)) return undefined;
    // Up to two quotes may end the content just before the closing three.
    let extra = 0;
    while (extra < 2 && this.#text[this.#index + 3 + extra] === quote) extra += 1;
    this.#index += 3 + extra;
    return quote.repeat(extra);
  }

  /** A single-line string closes at its quote and cannot run past the line. */
  #singleLineClosing(quote: string): string | undefined {
    const char = this.#text[this.#index];
    if (char === quote) {
      this.#index += 1;
      return "";
    }
    if (char === "\n" || char === "\r") this.fail("a string is not closed");
    return undefined;
  }

  /** The next character of a string's content, or what the escape there stands for in a basic (`"`) string. */
  #stringChar(quote: string, multiLine: boolean): string {
    const char = this.#text[this.#index]!;
    if (quote === "\"" && char === "\\") return this.#escape(multiLine);
    this.#index += 1;
    return char;
  }

  #escape(multiLine: boolean): string {
    const code = this.#text[this.#index + 1] ?? "";
    if (code in this.#escapes) {
      this.#index += 2;
      return this.#escapes[code]!;
    }
    if (code in this.#hexEscapes) return this.#codePoint(code, this.#hexEscapes[code]!);
    // A line-ending backslash in a multi-line basic string trims the line break and the blanks after it.
    const rest = /^\\[ \t]*\r?\n[\s]*/u.exec(this.#text.slice(this.#index));
    if (multiLine && rest !== null) {
      this.#index += rest[0].length;
      return "";
    }
    this.fail(`unknown escape \\${code}`);
  }

  /** The character a `\u`, `\U` or `\x` escape of `digits` hex digits names. */
  #codePoint(code: string, digits: number): string {
    const hex = this.#text.slice(this.#index + 2, this.#index + 2 + digits);
    if (!/^[0-9A-Fa-f]+$/u.test(hex) || hex.length !== digits) this.fail(`a \\${code} escape needs hex digits`);
    const point = Number.parseInt(hex, 16);
    if (point > 0x10ffff || (point >= 0xd800 && point <= 0xdfff)) this.fail(`a \\${code} escape is not a Unicode scalar value`);
    this.#index += 2 + digits;
    return String.fromCodePoint(point);
  }
}
