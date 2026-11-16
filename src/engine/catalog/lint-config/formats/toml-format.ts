/**
 * The TOML set-up reads for a repository's own lint config: the string lists, strings, scalars and tables Ruff and
 * Bandit read, and the tables Cargo and SQLFluff read whole. `toml-parser.ts` parses the syntax; this collects the
 * values at the wanted key paths and refuses what it cannot read for sure: a wanted key of another type, a key set
 * twice, or a table under a wanted array of tables. Each refusal throws `LintConfigFormatError` with the line.
 */
import { MAX_NESTING, TomlParser, type TomlHeader } from "./toml-parser.js";

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

/**
 * What `readTomlTrees` found: each wanted table the file sets, whole, each wanted array of tables, and every key path
 * the file sets.
 */
export interface TomlTrees {
  /**
   * Dotted table path (`lints`) → its keys (unquoted) → their values, a table however the file writes it (a header,
   * dotted keys, an inline table) as a `Map`. A wanted table the file does not set is absent.
   */
  readonly trees: ReadonlyMap<string, ReadonlyMap<string, TomlValue>>;
  /**
   * Dotted path (`bin`) → each element of the array of tables there, in order, as a `Map` of its keys, however the
   * file writes it (`[[bin]]` headers, or an array of inline tables). A wanted array the file does not set is absent.
   */
  readonly arrays: ReadonlyMap<string, readonly ReadonlyMap<string, TomlValue>[]>;
  readonly paths: readonly string[];
}

const BARE_KEY_TEXT = /^[A-Za-z0-9_-]+$/u;
/** A key path as TOML writes it: bare parts as they are, others quoted, so `"lint.ignore"` is not `lint.ignore`. */
const dottedPath = (parts: readonly string[]): string => parts.map(part => (BARE_KEY_TEXT.test(part) ? part : JSON.stringify(part))).join(".");

/**
 * The TOML a linter parses: 1.0 (Python's tomllib, as Bandit and SQLFluff read it) or 1.1, which Ruff and Cargo read
 * and which adds inline tables over several lines (comments, a trailing comma) and the `\e` and `\xHH` escapes.
 */
export type TomlVersion = "1.0" | "1.1";

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
  version: TomlVersion = "1.0",
): TomlStringLists {
  const found = new TomlReader(text, { wanted, wantedTables, wantedStrings, wantedScalars, wantedTrees: [], wantedArrays: [] }, version).read();
  return { lists: found.lists, tables: found.tables, strings: found.strings, scalars: found.scalars, paths: found.paths };
}

/**
 * The tables at `wanted` dotted paths, whole: every key under each, at any depth, with its value; and the arrays of
 * tables at `wantedArrays`, each element whole. A table under an element of a wanted array (`[bin.x]` after
 * `[[bin]]`) is refused. Reads and refuses as `readTomlStringLists` does.
 */
export function readTomlTrees(text: string, wanted: readonly string[], version: TomlVersion = "1.0", wantedArrays: readonly string[] = []): TomlTrees {
  const found = new TomlReader(text, { wanted: [], wantedTables: [], wantedStrings: [], wantedScalars: [], wantedTrees: wanted, wantedArrays }, version).read();
  return { trees: found.trees, arrays: found.arrays, paths: found.paths };
}

/** The key paths a `TomlReader` collects, by what it collects at each. */
interface TomlWanted {
  readonly wanted: readonly string[];
  readonly wantedTables: readonly string[];
  readonly wantedStrings: readonly string[];
  readonly wantedScalars: readonly string[];
  readonly wantedTrees: readonly string[];
  readonly wantedArrays: readonly string[];
}

class TomlReader {
  readonly #parser: TomlParser;
  #table: string[] | undefined = [];
  readonly #lists = new Map<string, string[]>();
  readonly #tables = new Map<string, Map<string, string[]>>();
  readonly #strings = new Map<string, string>();
  readonly #scalars = new Map<string, string>();
  readonly #trees = new Map<string, Map<string, TomlValue>>();
  readonly #arrays = new Map<string, Map<string, TomlValue>[]>();
  /** The element of a wanted array of tables whose keys follow, after its `[[…]]` header. */
  #element: Map<string, TomlValue> | undefined;
  readonly #paths = new Set<string>();
  /** The dotted paths of every array of tables (`[[a]]`), whose elements' keys and sub-tables (`[a.b]`) are read past. */
  readonly #arrayTables = new Set<string>();
  readonly wanted: ReadonlySet<string>;
  readonly wantedTables: ReadonlySet<string>;
  readonly wantedStrings: ReadonlySet<string>;
  readonly wantedScalars: ReadonlySet<string>;
  readonly wantedTrees: ReadonlySet<string>;
  readonly wantedArrays: ReadonlySet<string>;
  /**
   * The longest wanted tree path, the longest wanted list, string or scalar path, and the longest wanted array path:
   * see `dottedPrefixes`.
   */
  readonly #longestTree: number;
  readonly #longestKey: number;
  readonly #longestArray: number;

  constructor(text: string, wanted: TomlWanted, version: TomlVersion) {
    this.#parser = new TomlParser(text, version);
    this.wanted = new Set(wanted.wanted);
    this.wantedTables = new Set(wanted.wantedTables);
    this.wantedStrings = new Set(wanted.wantedStrings);
    this.wantedScalars = new Set(wanted.wantedScalars);
    this.wantedTrees = new Set(wanted.wantedTrees);
    this.wantedArrays = new Set(wanted.wantedArrays);
    this.#longestTree = Math.max(0, ...wanted.wantedTrees.map(path => path.length));
    this.#longestKey = Math.max(0, ...[...wanted.wanted, ...wanted.wantedStrings, ...wanted.wantedScalars].map(path => path.length));
    this.#longestArray = Math.max(0, ...wanted.wantedArrays.map(path => path.length));
  }

  read(): TomlStringLists & TomlTrees {
    while (this.#parser.nextLine()) {
      if (this.#parser.atTableHeader()) this.#header(this.#parser.tableHeader());
      else this.#keyValue();
      this.#parser.endOfLine();
    }
    return { lists: this.#lists, tables: this.#tables, strings: this.#strings, scalars: this.#scalars, trees: this.#trees, arrays: this.#arrays, paths: [...this.#paths] };
  }

  #fail(problem: string): never {
    return this.#parser.fail(problem);
  }

  #header({ path, arrayTable }: TomlHeader): void {
    this.#refuseTables(path, path.length);
    this.#element = this.#arrayHeader(path, arrayTable);
    // No wanted key lives in an array of tables, nor in a table under one (`[a.b]` after `[[a]]`, a table of its last
    // element): their keys are read past, never collected, but for a wanted array's.
    const inArray = arrayTable || this.#under(path, this.#arrayTables, Number.POSITIVE_INFINITY);
    this.#table = inArray ? undefined : path;
    if (!inArray) {
      this.#paths.add(dottedPath(path));
      this.#wantedTable(dottedPath(path));
    }
  }

  /**
   * Records array of tables header `[[path]]`; a new element of a wanted array, for the keys that follow, else
   * undefined. Refuses a table at a wanted array's path (`[bin]`, or `[[bin]]` after `bin = […]`) or under one
   * (`[bin.x]`).
   */
  #arrayHeader(path: readonly string[], arrayTable: boolean): Map<string, TomlValue> | undefined {
    const dotted = dottedPath(path);
    if (this.#under(path, this.wantedArrays, this.#longestArray)) this.#fail(`[${dotted}] is a table under an array of tables, which Rerail does not read`);
    if (arrayTable) this.#arrayTables.add(dotted);
    if (!this.wantedArrays.has(dotted)) return undefined;
    if (!arrayTable || this.#paths.has(dotted)) this.#fail(`${dotted} is not an array of tables`);
    return this.#newElement(dotted);
  }

  /** A new, empty element appended to the wanted array of tables at `dotted`. */
  #newElement(dotted: string): Map<string, TomlValue> {
    const element = new Map<string, TomlValue>();
    this.#arrays.set(dotted, [...(this.#arrays.get(dotted) ?? []), element]);
    return element;
  }

  /** Whether a table holding `path` (not `path` itself) is one of `paths`, none longer than `longest`. */
  #under(path: readonly string[], paths: ReadonlySet<string>, longest: number): boolean {
    for (const dotted of dottedPrefixes(path, path.length - 1, longest)) {
      if (paths.has(dotted)) return true;
    }
    return false;
  }

  /** A wanted table or tree, recorded even while it is empty: an empty table is set, not absent. */
  #wantedTable(dotted: string): void {
    if (this.wantedTables.has(dotted) && !this.#tables.has(dotted)) this.#tables.set(dotted, new Map());
    if (this.wantedTrees.has(dotted) && !this.#trees.has(dotted)) this.#trees.set(dotted, new Map());
  }

  #keyValue(): void {
    if (this.#element !== undefined) {
      this.#parser.inlineEntry(this.#element, 0);
      return;
    }
    const { key, value } = this.#parser.keyValue();
    if (this.#table !== undefined) this.#collect([...this.#table, ...key], value);
  }

  /** Records `value` at `path`; `depth` is how many tables of the key's value enclose it. */
  #collect(path: readonly string[], value: TomlValue, depth = 0): void {
    this.#refuseTables(path, value instanceof Map ? path.length : path.length - 1);
    const dotted = this.#mark(path, value);
    this.#collectArray(path, dotted, value);
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

  /** Records a wanted array of tables set as a value (`bin = [{ … }]`); refuses a key under one (`bin.path = …`). */
  #collectArray(path: readonly string[], dotted: string, value: TomlValue): void {
    if (this.#under(path, this.wantedArrays, this.#longestArray)) this.#fail(`${dotted} is set under an array of tables`);
    if (!this.wantedArrays.has(dotted)) return;
    if (this.#arrays.has(dotted)) this.#fail(`${dotted} is set twice`);
    if (!Array.isArray(value) || !value.every(item => item instanceof Map)) this.#fail(`${dotted} is not an array of tables`);
    this.#arrays.set(dotted, value as Map<string, TomlValue>[]);
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
    else this.#tableList(table, path.at(-1)!, value as string[]);
  }

  /** Records `list` at `key` of the wanted table at `table`. */
  #tableList(table: string, key: string, list: string[]): void {
    this.#tables.set(table, (this.#tables.get(table) ?? new Map<string, string[]>()).set(key, list));
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
}
