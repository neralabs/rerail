/**
 * A narrow reader of Rust source for the Ruff recorder: enough of Rust's lexical grammar to find the string literals,
 * use declarations, crate paths and test modules of upstream code written in Rust (Ruff), and a translation of the
 * regex crate's syntax for the patterns that code compiles. Comments, doc comments included, are not tokens; raw
 * strings, byte strings, char literals, lifetimes and nested block comments lex as Rust lexes them. Ported from
 * nera-rule-validation's src/validation/languages/rust.ts.
 */

import { regexWords } from "../gates/policy-literals.js";

export type RustTokenKind = "ident" | "string" | "char" | "lifetime" | "number" | "punct";

export interface RustToken {
  kind: RustTokenKind;
  text: string;
  /** The decoded value of a string or char literal (raw, byte and C strings included). */
  value?: string;
}

const SIMPLE_ESCAPES: Record<string, string> = { n: "\n", r: "\r", t: "\t", "0": "\0", "\\": "\\", "'": "'", '"': '"' };
const PUNCTUATION = ["<<=", ">>=", "..=", "...", "::", "->", "=>", "..", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "%=", "^=", "&=", "|=", "<<", ">>"];

/** The escape just after a backslash at `index` of a quoted literal: its value and how many characters it spans. */
function escape(source: string, index: number): { value: string; length: number } {
  const next = source[index] ?? "";
  if (next in SIMPLE_ESCAPES) return { value: SIMPLE_ESCAPES[next]!, length: 1 };
  const hex = /^x([0-9A-Fa-f]{2})/u.exec(source.slice(index, index + 3));
  if (hex !== null) return { value: String.fromCharCode(Number.parseInt(hex[1]!, 16)), length: 3 };
  const unicode = /^u\{([0-9A-Fa-f_]{1,8})\}/u.exec(source.slice(index, index + 12));
  if (unicode !== null) return { value: String.fromCodePoint(Number.parseInt(unicode[1]!.replaceAll("_", ""), 16)), length: unicode[0].length };
  if (next === "\n" || next === "\r") {
    // A string continuation: the backslash, the line break and the next line's leading whitespace are dropped.
    let length = 1;
    while (/\s/u.test(source[index + length] ?? "")) length += 1;
    return { value: "", length };
  }
  return { value: next, length: 1 };
}

/** Lex Rust source into tokens, dropping whitespace and every comment. */
export function lexRust(source: string): RustToken[] {
  const tokens: RustToken[] = [];
  let index = 0;
  while (index < source.length) {
    const char = source[index]!;
    if (/\s/u.test(char)) {
      index += 1;
      continue;
    }
    if (source.startsWith("//", index)) {
      while (index < source.length && source[index] !== "\n") index += 1;
      continue;
    }
    if (source.startsWith("/*", index)) {
      // Block comments nest in Rust.
      let depth = 0;
      do {
        if (source.startsWith("/*", index)) {
          depth += 1;
          index += 2;
        } else if (source.startsWith("*/", index)) {
          depth -= 1;
          index += 2;
        } else index += 1;
      } while (depth > 0 && index < source.length);
      continue;
    }
    // Raw strings: r"...", r#"..."#, br"...", cr#"..."#; no escapes, closed by a quote and the same number of hashes.
    const raw = /^[bc]?r(#*)"/u.exec(source.slice(index, index + 260));
    if (raw !== null) {
      const hashes = raw[1]!;
      const start = index + raw[0].length;
      const close = source.indexOf(`"${hashes}`, start);
      const end = close === -1 ? source.length : close;
      tokens.push({ kind: "string", text: source.slice(index, end + 1 + hashes.length), value: source.slice(start, end) });
      index = end + 1 + hashes.length;
      continue;
    }
    // Quoted strings, with an optional byte or C prefix.
    const quoted = /^[bc]?"/u.exec(source.slice(index, index + 2));
    if (quoted !== null) {
      let cursor = index + quoted[0].length;
      let value = "";
      while (cursor < source.length && source[cursor] !== '"') {
        if (source[cursor] === "\\") {
          const decoded = escape(source, cursor + 1);
          value += decoded.value;
          cursor += 1 + decoded.length;
        } else {
          value += source[cursor];
          cursor += 1;
        }
      }
      tokens.push({ kind: "string", text: source.slice(index, cursor + 1), value });
      index = cursor + 1;
      continue;
    }
    // A char or byte literal ('a', '\n', '\u{1F600}', b'x') or a lifetime ('a, 'static).
    const quote = char === "'" ? index : char === "b" && source[index + 1] === "'" ? index + 1 : -1;
    if (quote !== -1) {
      if (source[quote + 1] === "\\") {
        const decoded = escape(source, quote + 2);
        const end = quote + 2 + decoded.length;
        if (source[end] === "'") {
          tokens.push({ kind: "char", text: source.slice(index, end + 1), value: decoded.value });
          index = end + 1;
          continue;
        }
      } else {
        const codePoint = source.codePointAt(quote + 1);
        const width = codePoint === undefined ? 0 : codePoint > 0xffff ? 2 : 1;
        if (width > 0 && source[quote + 1 + width] === "'") {
          tokens.push({ kind: "char", text: source.slice(index, quote + 2 + width), value: source.slice(quote + 1, quote + 1 + width) });
          index = quote + 2 + width;
          continue;
        }
      }
      if (char === "'") {
        const lifetime = /^'[A-Za-z_]\w*/u.exec(source.slice(index, index + 256));
        if (lifetime !== null) {
          tokens.push({ kind: "lifetime", text: lifetime[0] });
          index += lifetime[0].length;
          continue;
        }
      }
    }
    const identifier = /^(?:r#)?[A-Za-z_]\w*/u.exec(source.slice(index, index + 256));
    if (identifier !== null) {
      tokens.push({ kind: "ident", text: identifier[0].replace(/^r#/u, "") });
      index += identifier[0].length;
      continue;
    }
    const number = /^\d\w*(?:\.(?![.A-Za-z_])\d*\w*)?/u.exec(source.slice(index, index + 64));
    if (number !== null) {
      tokens.push({ kind: "number", text: number[0] });
      index += number[0].length;
      continue;
    }
    const punct = PUNCTUATION.find((candidate) => source.startsWith(candidate, index)) ?? char;
    tokens.push({ kind: "punct", text: punct });
    index += punct.length;
  }
  return tokens;
}

const OPENERS = new Set(["(", "[", "{"]);
const CLOSERS = new Set([")", "]", "}"]);

/** The index of the bracket that closes the one at `open`; the last index when it is never closed. */
export function closingIndex(tokens: readonly RustToken[], open: number): number {
  let depth = 0;
  for (let index = open; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    if (token.kind !== "punct") continue;
    if (OPENERS.has(token.text)) depth += 1;
    else if (CLOSERS.has(token.text)) {
      depth -= 1;
      if (depth === 0) return index;
    }
  }
  return tokens.length - 1;
}

/** Split `tokens` on `separator` outside brackets. */
export function splitTopLevel(tokens: readonly RustToken[], separator: string): RustToken[][] {
  const parts: RustToken[][] = [];
  let current: RustToken[] = [];
  let depth = 0;
  for (const token of tokens) {
    if (token.kind === "punct") {
      if (OPENERS.has(token.text)) depth += 1;
      else if (CLOSERS.has(token.text)) depth -= 1;
      else if (depth === 0 && token.text === separator) {
        parts.push(current);
        current = [];
        continue;
      }
    }
    current.push(token);
  }
  if (current.length > 0) parts.push(current);
  return parts;
}

/** Whether the tokens at `index` are the attribute `#[cfg(test)]`. */
function isCfgTest(tokens: readonly RustToken[], index: number): boolean {
  return ["#", "[", "cfg", "(", "test", ")", "]"].every((text, offset) => tokens[index + offset]?.text === text);
}

/** `#![cfg(test)]`: an inner attribute that makes the rest of the enclosing file or module test-only. */
function isInnerCfgTest(tokens: readonly RustToken[], index: number): boolean {
  return ["#", "!", "[", "cfg", "(", "test", ")", "]"].every((text, offset) => tokens[index + offset]?.text === text);
}

/**
 * The tokens without any item marked `#[cfg(test)]` (a test module, a test-only function or import): code upstream
 * compiles only for its own tests is not what the rule checks.
 */
export function withoutTestItems(tokens: readonly RustToken[]): RustToken[] {
  const kept: RustToken[] = [];
  let depth = 0;
  for (let index = 0; index < tokens.length; index += 1) {
    if (isInnerCfgTest(tokens, index)) {
      // The rest of the file goes; inside a module, the rest of the module up to its closing brace, which stays.
      if (depth === 0) return kept;
      let inner = 0;
      for (index += 8; index < tokens.length; index += 1) {
        const token = tokens[index]!;
        if (token.kind === "punct" && token.text === "{") inner += 1;
        else if (token.kind === "punct" && token.text === "}") {
          if (inner === 0) break;
          inner -= 1;
        }
      }
      index -= 1;
      continue;
    }
    if (!isCfgTest(tokens, index)) {
      const token = tokens[index]!;
      if (token.kind === "punct" && token.text === "{") depth += 1;
      else if (token.kind === "punct" && token.text === "}") depth -= 1;
      kept.push(token);
      continue;
    }
    // Skip the attribute, any further attributes, then the item: up to a `;` or the end of its first top-level block.
    let cursor = index + 7;
    while (tokens[cursor]?.text === "#" && tokens[cursor + 1]?.text === "[") cursor = closingIndex(tokens, cursor + 1) + 1;
    while (cursor < tokens.length) {
      const token = tokens[cursor]!;
      if (token.kind === "punct" && token.text === ";") break;
      if (token.kind === "punct" && token.text === "{") {
        cursor = closingIndex(tokens, cursor);
        break;
      }
      if (token.kind === "punct" && (token.text === "(" || token.text === "[")) cursor = closingIndex(tokens, cursor);
      cursor += 1;
    }
    index = cursor;
  }
  return kept;
}

const IDENTIFIER = /^[A-Za-z_]\w*$/u;
/** A qualified name or path that upstream matches by its parts: `typing.List`, `/var/tmp`, `.run`. */
const SEGMENTED = /^(?:[./]|::)?[A-Za-z_]\w*(?:(?:[./]|::)[A-Za-z_]\w*)*$/u;
const QUOTED = /["'`]([A-Za-z_]\w*)["'`]/gu;

/** The names one string literal of upstream code checks: itself when identifier-shaped, its segments, its quoted words. */
export function literalWords(value: string): string[] {
  const words: string[] = [];
  if (IDENTIFIER.test(value)) words.push(value);
  else if (SEGMENTED.test(value)) words.push(...value.split(/[./]|::/u).filter((segment) => segment !== ""));
  for (const match of value.matchAll(QUOTED)) words.push(match[1]!);
  return words;
}

/**
 * The string literals a regex constructor compiles: the pattern of `Regex::new("...")` or `RegexBuilder::new("...")`,
 * and every pattern listed in `RegexSet::new([...])`.
 */
function regexPatterns(tokens: readonly RustToken[]): Set<number> {
  const found = new Set<number>();
  tokens.forEach((token, index) => {
    if (!/^Regex(?:Builder|Set)?$/u.test(token.text) || tokens[index + 1]?.text !== "::" || tokens[index + 2]?.text !== "new" || tokens[index + 3]?.text !== "(") return;
    if (token.text !== "RegexSet") {
      if (tokens[index + 4]?.kind === "string") found.add(index + 4);
      return;
    }
    const close = closingIndex(tokens, index + 3);
    for (let cursor = index + 4; cursor < close; cursor += 1) if (tokens[cursor]!.kind === "string") found.add(cursor);
  });
  return found;
}

/** The string literals a substring searcher looks for: the needle of memchr's `Finder::new(..)` or `FinderRev::new(..)`. */
function searchNeedles(tokens: readonly RustToken[]): Set<number> {
  const found = new Set<number>();
  tokens.forEach((token, index) => {
    if (/^Finder(?:Rev)?$/u.test(token.text) && tokens[index + 1]?.text === "::" && tokens[index + 2]?.text === "new"
      && tokens[index + 3]?.text === "(" && tokens[index + 4]?.kind === "string") found.add(index + 4);
  });
  return found;
}

export interface RustLiterals {
  /** Names the code checks: identifier-shaped string literals, segments of qualified names and paths, quoted words, words of regexes and search needles. */
  names: string[];
  /** The patterns it builds regexes from. */
  regexes: string[];
  /** Every path its `use` declarations import, as segments (`use a::{b, c::d}` gives `a::b` and `a::c::d`). */
  uses: string[][];
}

/** Expand one use tree (`a::{b, c::{d as e, self}, *}`, `::` separators removed) into the paths it imports. */
export function useTreePaths(tokens: readonly RustToken[], prefix: string[] = []): string[][] {
  const path = [...prefix];
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    if (token.kind === "ident") {
      // An alias renames the import; `self` imports the module the prefix names.
      if (token.text === "as" || tokens[index - 1]?.text === "as" || token.text === "self") continue;
      path.push(token.text);
    } else if (token.text === "{") {
      const close = closingIndex(tokens, index);
      return splitTopLevel(tokens.slice(index + 1, close), ",").flatMap((part) => useTreePaths(part, path));
    }
  }
  return path.length > 0 ? [path] : [];
}

/** Every path imported by the `use` declarations among `tokens`, at any nesting. */
function useDeclarations(tokens: readonly RustToken[]): string[][] {
  const paths: string[][] = [];
  for (let index = 0; index < tokens.length; index += 1) {
    if (tokens[index]!.kind !== "ident" || tokens[index]!.text !== "use") continue;
    let end = index + 1;
    while (end < tokens.length && tokens[end]!.text !== ";") end += 1;
    paths.push(...useTreePaths(tokens.slice(index + 1, end).filter((token) => token.text !== "::")));
    index = end;
  }
  return paths;
}

/** The literals and imports of one Rust source file, outside comments and `#[cfg(test)]` items. */
export function rustLiterals(source: string): RustLiterals {
  const tokens = withoutTestItems(lexRust(source));
  const names = new Set<string>();
  const regexes: string[] = [];
  const patterns = regexPatterns(tokens);
  const needles = searchNeedles(tokens);
  tokens.forEach((token, index) => {
    if (token.kind !== "string" || token.value === undefined) return;
    for (const word of literalWords(token.value)) names.add(word);
    if (patterns.has(index)) {
      regexes.push(token.value);
      for (const word of regexWords(token.value)) names.add(word);
    }
    // A substring search spells the text it looks for, as a regex does: `# /// script` checks the word `script`.
    if (needles.has(index)) for (const word of token.value.match(/[A-Za-z_]\w*/gu) ?? []) names.add(word);
  });
  return { names: [...names].sort(), regexes, uses: useDeclarations(tokens) };
}

const JS_SYNTAX = "^$\\.*+?()[]{}|/";

/**
 * A regex of Rust's `regex` crate as a JavaScript one (unicode mode), when it has a faithful translation: leading
 * `(?i)`, `(?m)` and `(?s)` become flags and `(?u)` (the crate's default) is dropped, `(?P<name>` becomes `(?<name>`,
 * `\pL` becomes `\p{L}`, `\x{…}` becomes `\u{…}`, and escaped punctuation JavaScript does not escape is written as a
 * code point. Undefined for anything else JavaScript lacks (other flags, mid-pattern flag groups, `\A`, `\z`, `\U`,
 * POSIX and nested classes, class set operations); an implementation regex that cannot run explains nothing.
 */
export function rustRegex(pattern: string): RegExp | undefined {
  let flags = "";
  let rest = pattern;
  for (let match = /^\(\?([A-Za-z]+)\)/u.exec(rest); match !== null; match = /^\(\?([A-Za-z]+)\)/u.exec(rest)) {
    for (const flag of match[1]!) {
      if (flag === "u") continue;
      if (!"ims".includes(flag)) return undefined;
      if (!flags.includes(flag)) flags += flag;
    }
    rest = rest.slice(match[0].length);
  }
  let source = "";
  let inClass = false;
  for (let index = 0; index < rest.length; index += 1) {
    const char = rest[index]!;
    const next = rest[index + 1];
    if (char === "\\") {
      if (next === undefined || "AzZU".includes(next)) return undefined;
      index += 1;
      if ((next === "p" || next === "P") && rest[index + 1] !== "{") {
        const name = rest[index + 1];
        if (name === undefined) return undefined;
        source += `\\${next}{${name}}`;
        index += 1;
      } else if (next === "x" && rest[index + 1] === "{") {
        source += "\\u";
      } else if (/[0-9A-Za-z]/u.test(next) || JS_SYNTAX.includes(next) || (inClass && next === "-")) {
        source += `\\${next}`;
      } else {
        source += `\\u{${next.codePointAt(0)!.toString(16)}}`;
      }
      continue;
    }
    if (inClass) {
      if (char === "[" || ((char === "&" || char === "~" || char === "-") && next === char)) return undefined;
      if (char === "]") inClass = false;
      source += char;
      continue;
    }
    if (char === "[") {
      inClass = true;
      source += char;
      if (rest[index + 1] === "^") {
        source += "^";
        index += 1;
      }
      // A `]` first in a Rust class is a literal.
      if (rest[index + 1] === "]") {
        source += "\\]";
        index += 1;
      }
      continue;
    }
    if (char === "(" && next === "?") {
      if (rest.startsWith("(?P<", index)) {
        source += "(?<";
        index += 3;
        continue;
      }
      if (!rest.startsWith("(?:", index) && !/^\(\?<[A-Za-z_]/u.test(rest.slice(index))) return undefined;
    }
    source += char;
  }
  try {
    return new RegExp(source, `${flags}u`);
  } catch {
    return undefined;
  }
}
