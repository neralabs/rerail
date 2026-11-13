/**
 * The lint levels a Rust crate root sets for its whole crate with inner attributes (`#![deny(clippy::pedantic)]`),
 * for set-up's reading of Clippy's config (`clippy-config.ts`). The file is read as tokens, never compiled: only the
 * inner attributes before its first item, and of those only `allow`, `warn`, `deny`, `forbid`, `expect` and the
 * `cfg_attr` that wrap them. It refuses what it cannot read rather than guess: it throws `LintConfigFormatError`.
 */
import { LintConfigFormatError } from "./formats/lint-config-formats.js";

/** A lint level an attribute sets; Cargo's `[lints]` takes all but `expect`. */
export type RustLintLevel = "allow" | "warn" | "deny" | "forbid" | "expect";

const LEVELS: Readonly<Record<string, RustLintLevel>> = { allow: "allow", warn: "warn", deny: "deny", forbid: "forbid", expect: "expect" };

/** One lint-level attribute at a crate root. */
export interface RustLintAttribute {
  readonly level: RustLintLevel;
  /** The lints and groups it names, as written (`clippy::needless_else`, `warnings`). */
  readonly names: readonly string[];
  /** A `cfg_attr` wraps it whose condition Rerail cannot decide: it may or may not apply. */
  readonly conditional: boolean;
  /** The inner attribute it comes from, as the file writes it on one line: `#![allow(clippy::needless_else)]`. */
  readonly text: string;
}

type Delimiter = "(" | "[" | "{";
/** A token, or a bracketed group of them with the source it spans. */
type TokenTree = { readonly token: string } | { readonly delimiter: Delimiter; readonly trees: readonly TokenTree[]; readonly source: string };
type Group = Extract<TokenTree, { delimiter: Delimiter }>;

const CLOSERS: Readonly<Record<Delimiter, string>> = { "(": ")", "[": "]", "{": "}" };
/** How deep brackets may nest: each is read recursively, so a deeper file would overflow the stack. */
const MAX_NESTING = 64;
/**
 * One token: a raw string (`r#"…"#`, byte and C forms too), a string, a character, a lifetime, an identifier or
 * number, `::`, or any other single character. An unclosed literal leaves its quote alone, which is refused.
 */
const TOKEN = /[bc]?r(#*)"[\s\S]*?"\1|[bc]?"(?:\\[\s\S]|[^"\\])*"|b?'(?:\\[^']+|[^\\'])'|'[\p{L}_][\p{L}\p{N}_]*|[\p{L}\p{N}_]+|::|\S/uy;

/** The lint-level attributes among the inner attributes of crate root `source`, in order. */
export function readRustLintAttributes(source: string): RustLintAttribute[] {
  return new RustTokenReader(source).innerAttributes().flatMap(group => lintAttributes(group.trees, false, `#!${group.source}`.replace(/\s+/gu, " ")));
}

const isToken = (tree: TokenTree | undefined, token: string): boolean => tree !== undefined && "token" in tree && tree.token === token;
const isGroup = (tree: TokenTree | undefined, delimiter: Delimiter): tree is Group => tree !== undefined && "delimiter" in tree && tree.delimiter === delimiter;

class RustTokenReader {
  #index: number;

  constructor(readonly text: string) {
    this.#index = text.startsWith("\uFEFF") ? 1 : 0;
  }

  /** The bracketed group of each inner attribute (`#![…]`) before the file's first item. */
  innerAttributes(): Group[] {
    this.#skipShebang();
    const groups: Group[] = [];
    for (let group = this.#innerAttribute(); group !== undefined; group = this.#innerAttribute()) groups.push(group);
    return groups;
  }

  /** The bracketed group of the inner attribute next in the file; undefined when something else is next. */
  #innerAttribute(): Group | undefined {
    if (!isToken(this.#tree(0), "#") || !isToken(this.#tree(0), "!")) return undefined;
    const group = this.#tree(0);
    if (!isGroup(group, "[")) this.#fail("expected [ after #!");
    return group;
  }

  #fail(problem: string): never {
    const line = this.text.slice(0, this.#index).split("\n").length;
    throw new LintConfigFormatError(`line ${line}: ${problem}`);
  }

  /**
   * A first line `#!…` whose next token past whitespace and comments is not `[` is a shebang, not an attribute
   * (rustc_lexer `strip_shebang`).
   */
  #skipShebang(): void {
    const start = this.#index;
    if (!this.text.startsWith("#!", start)) return;
    this.#index += 2;
    this.#skipBlank();
    const attribute = this.text[this.#index] === "[";
    this.#index = start;
    if (!attribute) this.#skipLineComment();
  }

  /** Whitespace and comments, block comments nesting. */
  #skipBlank(): void {
    for (;;) {
      while (/\s/u.test(this.text[this.#index] ?? "")) this.#index += 1;
      if (this.text.startsWith("//", this.#index)) this.#skipLineComment();
      else if (this.text.startsWith("/*", this.#index)) this.#skipBlockComment();
      else return;
    }
  }

  #skipLineComment(): void {
    const end = this.text.indexOf("\n", this.#index);
    this.#index = end < 0 ? this.text.length : end;
  }

  #skipBlockComment(): void {
    let depth = 0;
    const comment = /\/\*|\*\//gu;
    comment.lastIndex = this.#index;
    for (let match = comment.exec(this.text); match !== null; match = comment.exec(this.text)) {
      depth += match[0] === "/*" ? 1 : -1;
      if (depth === 0) {
        this.#index = comment.lastIndex;
        return;
      }
    }
    this.#fail("a block comment is not closed");
  }

  /** The next token or bracketed group, `depth` groups in; undefined at the end of the file. */
  #tree(depth: number): TokenTree | undefined {
    this.#skipBlank();
    if (this.#index >= this.text.length) return undefined;
    const char = this.text[this.#index]!;
    if (char in CLOSERS) return this.#group(char as Delimiter, depth);
    TOKEN.lastIndex = this.#index;
    const token = TOKEN.exec(this.text)![0];
    if (token === "\"" || token === "'") this.#fail("a literal is not closed");
    this.#index = TOKEN.lastIndex;
    return { token };
  }

  #group(delimiter: Delimiter, depth: number): Group {
    if (depth >= MAX_NESTING) this.#fail(`brackets nest more than ${MAX_NESTING} deep`);
    const start = this.#index;
    this.#index += 1;
    const trees: TokenTree[] = [];
    for (;;) {
      this.#skipBlank();
      if (this.text[this.#index] === CLOSERS[delimiter]) break;
      trees.push(this.#member(delimiter, depth));
    }
    this.#index += 1;
    return { delimiter, trees, source: this.text.slice(start, this.#index) };
  }

  /** The next tree inside a group opened by `delimiter`, `depth` groups in; refuses the end of the file or another closer. */
  #member(delimiter: Delimiter, depth: number): TokenTree {
    const tree = this.#tree(depth + 1);
    if (tree === undefined || ("token" in tree && ")]}".includes(tree.token))) this.#fail(`a ${delimiter} is not closed`);
    return tree;
  }
}

/** `trees` split at their top-level commas, a trailing comma allowed. */
function splitAtCommas(trees: readonly TokenTree[]): TokenTree[][] {
  const items: TokenTree[][] = [[]];
  for (const tree of trees) {
    if (isToken(tree, ",")) items.push([]);
    else items.at(-1)!.push(tree);
  }
  return items.filter(item => item.length > 0);
}

/** A path's parts alternate: an identifier, then `::`. */
const PATH_PARTS = [/^[\p{L}_][\p{L}\p{N}_]*$/u, /^::$/u] as const;

/** The path `trees` start with (`clippy::needless_else`) and how many trees it takes; `""` when they start with none. */
function leadingPath(trees: readonly TokenTree[]): { path: string; length: number } {
  const parts: string[] = [];
  for (const tree of trees) {
    if (!("token" in tree) || !PATH_PARTS[parts.length % 2]!.test(tree.token)) break;
    parts.push(tree.token);
  }
  // A path does not end in `::`.
  if (parts.length % 2 === 0) parts.pop();
  return { path: parts.join(""), length: parts.length };
}

/** The lint-level attributes attribute `trees` (the inside of `#![…]`) sets, `conditional` when a `cfg_attr` wraps it. */
function lintAttributes(trees: readonly TokenTree[], conditional: boolean, text: string): RustLintAttribute[] {
  const { path, length } = leadingPath(trees);
  const args = trees[length];
  // Any other attribute (`doc`, `no_std`, `recursion_limit = …`) sets no lint level.
  if (trees.length !== length + 1 || !isGroup(args, "(")) return [];
  const level = Object.hasOwn(LEVELS, path) ? LEVELS[path] : undefined;
  if (level !== undefined) return [{ level, names: lintNames(args.trees, text), conditional, text }];
  return path === "cfg_attr" ? cfgAttrLintAttributes(args.trees, conditional, text) : [];
}

/** The lint-level attributes a `cfg_attr(<predicate>, <attribute>, …)` with arguments `trees` sets. */
function cfgAttrLintAttributes(trees: readonly TokenTree[], conditional: boolean, text: string): RustLintAttribute[] {
  const [predicate, ...attributes] = splitAtCommas(trees);
  // Clippy sets `--cfg clippy`, so `cfg_attr(clippy, …)` always applies when Clippy lints.
  const always = !conditional && predicate?.length === 1 && isToken(predicate[0], "clippy");
  return attributes.flatMap(attribute => lintAttributes(attribute, !always, text));
}

/** The lints a lint-level attribute names, past its `reason = "…"`. */
function lintNames(trees: readonly TokenTree[], text: string): string[] {
  return splitAtCommas(trees).flatMap(item => {
    if (isToken(item[0], "reason") && isToken(item[1], "=")) return [];
    const { path, length } = leadingPath(item);
    if (length === 0 || length !== item.length) throw new LintConfigFormatError(`${text} names something that is not a lint`);
    return [path];
  });
}
