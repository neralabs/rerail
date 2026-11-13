/**
 * The small readers set-up needs for a repository's own lint config (`repository-lint-config.ts`): JSON with
 * comments and trailing commas, as ESLint's and oxlint's JSON config files allow, and the string lists, scalars and
 * tables Ruff and Cargo read from TOML. Each refuses what it cannot read rather than guess: it throws
 * `LintConfigFormatError`.
 */

export class LintConfigFormatError extends Error {}

/** `read()`'s value, or why the file is not read when a format reader refuses it; any other error is a bug and propagates. */
export function readFormat<T>(read: () => T): { readonly ok: true; readonly value: T } | { readonly ok: false; readonly why: string } {
  try {
    return { ok: true, value: read() };
  } catch (error) {
    if (error instanceof LintConfigFormatError) return { ok: false, why: error.message };
    throw error;
  }
}

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

/** A TOML value as the reader keeps it: strings, arrays and tables; any other scalar as its text, unread. */
export type TomlValue = string | TomlValue[] | Map<string, TomlValue> | { readonly scalar: string };

/** What `readTomlStringLists` found: the wanted lists, tables, strings and scalars, and every key path the file sets. */
export interface TomlStringLists {
  /** Dotted key path (`tool.ruff.lint.ignore`) → its list, for each wanted path the file sets. */
  readonly lists: ReadonlyMap<string, readonly string[]>;
  /** Dotted table path (`tool.ruff.lint.per-file-ignores`) → each key it sets (unquoted) → its list, for each wanted table. */
  readonly tables: ReadonlyMap<string, ReadonlyMap<string, readonly string[]>>;
  /** Dotted key path (`tool.ruff.extend`) → its string, for each wanted string path the file sets. */
  readonly strings: ReadonlyMap<string, string>;
  /** Dotted key path (`tool.ruff.preview`) → its scalar's text (`true`), for each wanted scalar path the file sets. */
  readonly scalars: ReadonlyMap<string, string>;
  /** Every key path the file sets, table headers included. */
  readonly paths: readonly string[];
}

/** What `readTomlTrees` found: each wanted table the file sets, whole, and every key path the file sets. */
export interface TomlTrees {
  /**
   * Dotted table path (`lints`) → its keys (unquoted) → their values, a table however the file writes it (a header,
   * dotted keys, an inline table) as a `Map`. A wanted table the file does not set is absent.
   */
  readonly trees: ReadonlyMap<string, ReadonlyMap<string, TomlValue>>;
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
 * The dotted paths of the first 1, 2, … `count` parts of `path`, each grown from the one before, while no longer than
 * `longest`: no longer one can be a wanted path, so a long dotted key costs its length, not its length squared.
 */
function* dottedPrefixes(path: readonly string[], count: number, longest: number): Generator<string> {
  let dotted = "";
  for (const part of path.slice(0, count)) {
    dotted += dotted === "" ? dottedPath([part]) : `.${dottedPath([part])}`;
    if (dotted.length > longest) return;
    yield dotted;
  }
}

/**
 * A TOML reader for exactly what set-up reads: the string lists at `wanted` key paths, in the `wantedTables` tables
 * every key's string list, the strings at `wantedStrings` and the scalars (numbers, booleans, dates) at
 * `wantedScalars`. It parses the whole file's structure (tables, dotted and quoted keys, strings of each kind, arrays
 * over several lines with comments and trailing commas, inline tables) so another key's value cannot be mistaken for
 * one of them, and keeps other values unread. Throws `LintConfigFormatError` (with the line) on what it cannot read,
 * a wanted key of another type, or a key set twice.
 */
export function readTomlStringLists(
  text: string,
  wanted: readonly string[],
  wantedTables: readonly string[] = [],
  wantedStrings: readonly string[] = [],
  wantedScalars: readonly string[] = [],
): TomlStringLists {
  const found = new TomlReader(text, { wanted, wantedTables, wantedStrings, wantedScalars, wantedTrees: [] }).read();
  return { lists: found.lists, tables: found.tables, strings: found.strings, scalars: found.scalars, paths: found.paths };
}

/**
 * The tables at `wanted` dotted paths, whole: every key under each, at any depth, with its value. Reads and refuses
 * as `readTomlStringLists` does.
 */
export function readTomlTrees(text: string, wanted: readonly string[]): TomlTrees {
  const found = new TomlReader(text, { wanted: [], wantedTables: [], wantedStrings: [], wantedScalars: [], wantedTrees: wanted }).read();
  return { trees: found.trees, paths: found.paths };
}

/** The key paths a `TomlReader` collects, by what it collects at each. */
interface TomlWanted {
  readonly wanted: readonly string[];
  readonly wantedTables: readonly string[];
  readonly wantedStrings: readonly string[];
  readonly wantedScalars: readonly string[];
  readonly wantedTrees: readonly string[];
}

class TomlReader {
  #index = 0;
  #table: string[] | undefined = [];
  readonly #lists = new Map<string, string[]>();
  readonly #tables = new Map<string, Map<string, string[]>>();
  readonly #strings = new Map<string, string>();
  readonly #scalars = new Map<string, string>();
  readonly #trees = new Map<string, Map<string, TomlValue>>();
  readonly #paths = new Set<string>();
  readonly wanted: ReadonlySet<string>;
  readonly wantedTables: ReadonlySet<string>;
  readonly wantedStrings: ReadonlySet<string>;
  readonly wantedScalars: ReadonlySet<string>;
  readonly wantedTrees: ReadonlySet<string>;
  /** The longest wanted tree path, and the longest wanted list, string or scalar path: see `dottedPrefixes`. */
  readonly #longestTree: number;
  readonly #longestKey: number;

  constructor(readonly text: string, wanted: TomlWanted) {
    this.wanted = new Set(wanted.wanted);
    this.wantedTables = new Set(wanted.wantedTables);
    this.wantedStrings = new Set(wanted.wantedStrings);
    this.wantedScalars = new Set(wanted.wantedScalars);
    this.wantedTrees = new Set(wanted.wantedTrees);
    this.#longestTree = Math.max(0, ...wanted.wantedTrees.map(path => path.length));
    this.#longestKey = Math.max(0, ...[...wanted.wanted, ...wanted.wantedStrings, ...wanted.wantedScalars].map(path => path.length));
  }

  read(): TomlStringLists & TomlTrees {
    if (this.text.startsWith("\uFEFF")) this.#index = 1;
    for (;;) {
      this.#skipBlank(true);
      if (this.#index >= this.text.length) break;
      if (this.text[this.#index] === "[") this.#header();
      else this.#keyValue();
      this.#endOfLine();
    }
    return { lists: this.#lists, tables: this.#tables, strings: this.#strings, scalars: this.#scalars, trees: this.#trees, paths: [...this.#paths] };
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
    const close = arrayTable ? "]]" : "]";
    this.#index += close.length;
    const path = this.#key();
    if (!this.text.startsWith(close, this.#index)) this.#fail("a table header is not closed");
    this.#index += close.length;
    this.#refuseTables(path, path.length);
    // No wanted key lives in an array of tables: its keys are read past, never collected.
    this.#table = arrayTable ? undefined : path;
    if (!arrayTable) {
      this.#paths.add(dottedPath(path));
      this.#wantedTable(dottedPath(path));
    }
  }

  /** A wanted table or tree, recorded even while it is empty: an empty table is set, not absent. */
  #wantedTable(dotted: string): void {
    if (this.wantedTables.has(dotted) && !this.#tables.has(dotted)) this.#tables.set(dotted, new Map());
    if (this.wantedTrees.has(dotted) && !this.#trees.has(dotted)) this.#trees.set(dotted, new Map());
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
    this.#refuseTables(path, value instanceof Map ? path.length : path.length - 1);
    const dotted = this.#mark(path, value);
    if (value instanceof Map) {
      this.#collectTable(path, dotted, value, depth);
      return;
    }
    this.#plant(path, value);
    if (this.wantedTables.has(dotted)) this.#fail(`${dotted} is not a table`);
    if (this.wantedStrings.has(dotted)) this.#collectString(dotted, value);
    else if (this.wantedScalars.has(dotted)) this.#collectScalar(dotted, value);
    else this.#collectList(path, dotted, value);
  }

  /** Records the table `value` at `path` and each of its keys, `depth` tables in. */
  #collectTable(path: readonly string[], dotted: string, value: Map<string, TomlValue>, depth: number): void {
    if (depth >= MAX_NESTING) this.#fail(`values nest more than ${MAX_NESTING} deep`);
    this.#wantedTable(dotted);
    for (const [key, inner] of value) this.#collect([...path, key], inner, depth + 1);
  }

  /** Marks `path` set, refusing a value set twice (a table may be added to); its dotted path. */
  #mark(path: readonly string[], value: TomlValue): string {
    const dotted = dottedPath(path);
    if (this.#paths.has(dotted) && !(value instanceof Map)) this.#fail(`${dotted} is set twice`);
    this.#paths.add(dotted);
    return dotted;
  }

  #collectString(dotted: string, value: TomlValue): void {
    if (typeof value !== "string") this.#fail(`${dotted} is not a string`);
    this.#strings.set(dotted, value);
  }

  #collectScalar(dotted: string, value: Exclude<TomlValue, Map<string, TomlValue>>): void {
    if (typeof value === "string" || Array.isArray(value)) this.#fail(`${dotted} is not a scalar`);
    this.#scalars.set(dotted, value.scalar);
  }

  #collectList(path: readonly string[], dotted: string, value: TomlValue): void {
    const table = dottedPath(path.slice(0, -1));
    if (!this.wanted.has(dotted) && !this.wantedTables.has(table)) return;
    if (!Array.isArray(value) || !value.every(item => typeof item === "string")) this.#fail(`${dotted} is not a list of strings`);
    if (this.wanted.has(dotted)) this.#lists.set(dotted, value as string[]);
    else this.#tables.set(table, (this.#tables.get(table) ?? new Map<string, string[]>()).set(path.at(-1)!, value as string[]));
  }

  /** Puts the value at `path` (not a table) into the wanted tree it lies under, if any, nesting a `Map` per table. */
  #plant(path: readonly string[], value: TomlValue): void {
    const root = this.#treeRoot(path);
    if (root < 0) return;
    let target = this.#trees.get(dottedPath(path.slice(0, root))) ?? new Map<string, TomlValue>();
    this.#trees.set(dottedPath(path.slice(0, root)), target);
    for (const part of path.slice(root, -1)) {
      const inner = target.get(part) ?? new Map<string, TomlValue>();
      if (!(inner instanceof Map)) this.#fail(`${dottedPath(path)} is set under a value that is not a table`);
      target.set(part, inner);
      target = inner;
    }
    target.set(path.at(-1)!, value);
  }

  /** How many parts of `path` the wanted tree it lies under has, the shortest first; -1 when it lies under none. */
  #treeRoot(path: readonly string[]): number {
    let parts = 0;
    for (const dotted of dottedPrefixes(path, path.length - 1, this.#longestTree)) {
      parts += 1;
      if (this.wantedTrees.has(dotted)) return parts;
    }
    return -1;
  }

  /** Refuses a wanted list, string or scalar among the first `count` parts of `path`: those parts are tables. */
  #refuseTables(path: readonly string[], count: number): void {
    for (const dotted of dottedPrefixes(path, count, this.#longestKey)) {
      const kind = this.#kindAt(dotted);
      if (kind !== undefined) this.#fail(`${dotted} is not ${kind}`);
    }
  }

  /** What the wanted key at `dotted` must be, as a refusal names it; undefined when no list, string or scalar is wanted there. */
  #kindAt(dotted: string): string | undefined {
    if (this.wanted.has(dotted)) return "a list of strings";
    if (this.wantedStrings.has(dotted)) return "a string";
    return this.wantedScalars.has(dotted) ? "a scalar" : undefined;
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
