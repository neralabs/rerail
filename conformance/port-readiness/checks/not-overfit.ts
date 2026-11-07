import { affixedAlternations, codeWords, policyLiterals, policyRegexes, regexWords, type PolicyLiteral, type PolicyRegex } from "../policy-literals.js";
import type { Check } from "../types.js";

export type LiteralClass = "language" | "implementation" | "shared-implementation" | "fixture-only" | "scaffold" | "unexplained";

export interface ClassifiedLiteral extends PolicyLiteral {
  class: LiteralClass;
}

/** What upstream says about names, and how to find the node a pattern context selects. */
export interface NotOverfitInputs {
  /** Keywords of the upstream language: syntax in every role. */
  keywords: Set<string>;
  /** Words the language gives meaning to inside comments (documentation tags, directives): syntax only in regexes. */
  commentVocabulary: Set<string>;
  /** Names the rule's own upstream implementation checks. */
  implementation: Set<string>;
  /** Names the shared rule helpers it delegates to check. */
  sharedImplementation: Set<string>;
  /** Regexes upstream's implementation matches names with; the few test words each selects are names upstream checks. */
  implementationRegexes: RegExp[];
  /** Every word in upstream's test files and test configurations. */
  fixtures: Set<string>;
  /** The text of every upstream test file, which regexes are run over. */
  fixtureTexts: string[];
  /** The source text of the node a pattern context selects, or undefined when the engine cannot find it. */
  selectedText(context: string, selector: string, language: string): string | undefined;
}

export interface NotOverfitResult {
  checks: Check[];
  literals: ClassifiedLiteral[];
  /** Regexes the gate could not run as JavaScript regular expressions; the gate fails rather than pass them unjudged. */
  unchecked: string[];
}

/** A regex that touches more distinct unexplained test identifiers than this is a syntax pattern, not a list of names. */
export const REGEX_NAME_LIMIT = 10;

/**
 * A test text this long holding fewer than one identifier per SPARSE_TEXT characters is a stress test of upstream's
 * own engine, which Rerail does not run: not-overfit does not read it. Of every gated tool-pack's test texts, only
 * ESLint's five qualify (100,000 to 1,000,011 characters with one or two identifiers, none of them unique to these
 * texts apart from the 100,000- and 200,000-letter `aaaa…` ones); every other text over 20,000 characters holds at least
 * one identifier per 16. A node regex such as `^[^\[]*[\w$]\[`, tried at every offset of a 1,000,000-backslash string,
 * backtracks across all of it each time: reading them cost a cold javascript/eslint shard half an hour.
 */
const LONG_TEXT = 20_000;
const SPARSE_TEXT = 1_000;

interface Identifier { start: number; end: number; word: string }
interface PreparedText { text: string; identifiers: Identifier[] }

/**
 * Upstream's test texts without its stress tests, each with its identifiers in order: computed once per list, since
 * every regex of every rule reads the same texts (33,511 for ESLint) and scanning them again per regex cost more than
 * the regexes did.
 */
const preparedTexts = new WeakMap<readonly string[], PreparedText[]>();
function prepareTexts(texts: readonly string[]): PreparedText[] {
  let prepared = preparedTexts.get(texts);
  if (prepared === undefined) {
    prepared = texts
      .map((text) => ({ text, identifiers: [...text.matchAll(IDENTIFIER)].map((match) => ({ start: match.index, end: match.index + match[0].length, word: match[0] })) }))
      .filter(({ text, identifiers }) => text.length <= LONG_TEXT || identifiers.length * SPARSE_TEXT >= text.length);
    preparedTexts.set(texts, prepared);
  }
  return prepared;
}

/** The index of the first identifier ending after `offset`; a text's identifiers are in order and never overlap. */
function firstEndingAfter(identifiers: readonly Identifier[], offset: number): number {
  let low = 0;
  let high = identifiers.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (identifiers[middle]!.end <= offset) low = middle + 1;
    else high = middle;
  }
  return low;
}

const POSIX_CLASSES: Record<string, string> = {
  alnum: "A-Za-z0-9", alpha: "A-Za-z", digit: "0-9", lower: "a-z", upper: "A-Z", space: "\\s", word: "\\w", xdigit: "0-9A-Fa-f", punct: "!-\\/:-@\\[-`{-~",
};

/**
 * Rust regex syntax that JavaScript's unicode mode spells differently: a one-letter class (`\pL` is `\p{L}`), an
 * ASCII word boundary (`(?-u:\b)` is JavaScript's own `\b`), Unicode mode switched on (`(?u)`, `(?u:x)`; JavaScript
 * runs in it already), a `]` first in a class (`[]a]`), and a `}` that closes no repetition (`^}$`).
 */
function rustSpellings(body: string): string {
  let out = "";
  let inClass = false;
  for (let index = 0; index < body.length; index += 1) {
    const char = body[index]!;
    const rest = body.slice(index);
    if (char === "\\") {
      // An escape is read whole, braces included (`\p{XID_Start}`, `\x{200C}`), so its `}` is not a lone one.
      const escape = /^\\(?:[pPxu]\{[^}]*\}|[pP][A-Za-z]|[\s\S])?/u.exec(rest)![0];
      out += /^\\[pP][A-Za-z]$/u.test(escape) ? `\\${escape[1]}{${escape[2]}}` : escape;
      index += escape.length - 1;
      continue;
    }
    if (inClass) {
      inClass = char !== "]";
      out += char;
      continue;
    }
    const opening = /^\[\^?\]?/u.exec(rest);
    const boundary = /^\(\?-u:\\([bB])\)/u.exec(rest);
    const unicode = /^\(\?u\)|^\(\?([a-tv-zA-Z]*)u([a-zA-Z]*):/u.exec(rest);
    const repetition = /^\{\d+(?:,\d*)?\}/u.exec(rest);
    if (char === "[") {
      inClass = true;
      out += opening![0].endsWith("]") ? `${opening![0].slice(0, -1)}\\]` : opening![0];
      index += opening![0].length - 1;
    } else if (boundary !== null) {
      out += `\\${boundary[1]}`;
      index += boundary[0].length - 1;
    } else if (unicode !== null) {
      out += unicode[0] === "(?u)" ? "" : `(?${unicode[1]}${unicode[2]}:`;
      index += unicode[0].length - 1;
    } else if (repetition !== null) {
      out += repetition[0];
      index += repetition[0].length - 1;
    } else {
      out += char === "}" ? "\\}" : char;
    }
  }
  return out;
}

/**
 * Translate the Rust regex syntax policies use into a JavaScript RegExp with the same meaning, or undefined when the
 * gate cannot run it faithfully (verbose mode, inline flags after the start, syntax JavaScript rejects).
 */
export function javascriptRegex(pattern: string, extraFlags = ""): RegExp | undefined {
  const inline = /^\(\?([imsxu]+)\)/u.exec(pattern);
  if (inline?.[1]?.includes("x") === true) return undefined;
  const flags = (inline?.[1] ?? "").replaceAll("u", "");
  const body = rustSpellings(pattern.slice(inline?.[0].length ?? 0));
  const unescaped = body.replaceAll("\\\\", "");
  // JavaScript has scoped modifier groups for i, m and s, but no flag switches mid-pattern and no verbose mode.
  if (/\(\?[a-zA-Z-]+\)/u.test(unescaped) || [...unescaped.matchAll(/\(\?([a-zA-Z-]+):/gu)].some((group) => /[^ims-]/u.test(group[1]!))) return undefined;
  const translated = body
    .replaceAll("(?P<", "(?<")
    .replaceAll("\\A", "^")
    .replaceAll("\\z", "$")
    .replace(/\\x\{([0-9A-Fa-f]+)\}/gu, "\\u{$1}");
  // Rust's class intersection and difference (`[\s&&[^\xA0]]`) are JavaScript's in unicodeSets mode, which spells
  // classes differently (unicodeSetsSource); Rust's symmetric difference (`~~`) JavaScript lacks.
  const sets = unicodeSetsSource(translated);
  if (sets === null) return undefined;
  const source = sets ?? translated
    // Rust lets any punctuation be escaped; JavaScript's unicode mode accepts escapes only for syntax characters. Escapes
    // are read in pairs, so the quote after an escaped backslash (`\\'`) stays a quote.
    .replace(/\\(.)/gsu, (whole, char: string) => (UNESCAPED_PUNCTUATION.includes(char) ? char : whole))
    .replace(/\[:(\^?)([a-z]+):\]/gu, (whole, negated: string, name: string) => (negated === "" && POSIX_CLASSES[name] !== undefined ? POSIX_CLASSES[name]! : whole));
  try {
    return new RegExp(source, `${[...new Set(`${flags}${extraFlags}`)].join("")}${sets === undefined ? "u" : "v"}`);
  } catch {
    return undefined;
  }
}

/** Punctuation Rust lets a pattern escape that JavaScript's unicode mode rejects escaped. */
const UNESCAPED_PUNCTUATION = `!"#%&',:;<=>@_\`~ `;
/** Characters unicodeSets mode reads as syntax inside a class: escaped when a class names them. */
const SETS_SYNTAX = "()[]{}/-\\|&!#$%*+,.:;<=>?@^`~";
/** Rust's one-letter escapes for a character, with JavaScript's spelling. */
const CHARACTER_ESCAPES: Record<string, string> = { a: "\\x07", f: "\\f", n: "\\n", r: "\\r", t: "\\t", v: "\\v" };

/** One member of a Rust class: a character (a range endpoint), a set of them, or a Rust set operator. */
type ClassMember = { kind: "character" | "set" | "union"; text: string } | { kind: "operator"; text: "&&" | "--" };

/**
 * A Rust pattern whose classes use intersection or difference, spelled for JavaScript's unicodeSets mode: undefined when
 * no class uses one (the pattern stays in unicode mode, as written), null when it cannot be spelled exactly.
 *
 * Rust binds a class's union tighter than its operators, which share one precedence and apply left to right
 * (`[\p{L}_--\p{Lu}--\p{Lt}]` is `[[\p{L}_]--\p{Lu}--\p{Lt}]`, `[a-z--[aeiou]&&\p{Lower}]` is
 * `[[[a-z]--[aeiou]]&&\p{Lower}]`). unicodeSets mode takes one operator kind per class and only single operands, so each
 * union or range operand is bracketed and a change of operator nests what came before; it also reads more punctuation as
 * syntax inside a class, so every literal one is escaped.
 */
function unicodeSetsSource(pattern: string): string | undefined | null {
  let at = 0;
  let operation = false;

  const character = (): { kind: "character" | "set"; text: string } | null => {
    if (pattern[at] !== "\\") {
      const char = String.fromCodePoint(pattern.codePointAt(at)!);
      at += char.length;
      return { kind: "character", text: SETS_SYNTAX.includes(char) ? `\\${char}` : char };
    }
    const escape = /^\\(?:[pP]\{[^}]*\}|u\{[0-9A-Fa-f]+\}|u[0-9A-Fa-f]{4}|x[0-9A-Fa-f]{2}|[\s\S])/u.exec(pattern.slice(at))![0];
    at += escape.length;
    const letter = escape[1]!;
    if (/^[pPdDwWsS]$/u.test(letter)) return { kind: "set", text: escape };
    if (/^[ux]$/u.test(letter) && escape.length > 2) return { kind: "character", text: escape };
    if (CHARACTER_ESCAPES[letter] !== undefined) return { kind: "character", text: CHARACTER_ESCAPES[letter]! };
    if (/^[\p{L}\p{N}]$/u.test(letter)) return null;
    return { kind: "character", text: SETS_SYNTAX.includes(letter) ? escape : letter };
  };

  const operand = (members: ClassMember[]): string | null => {
    if (members.length === 0) return null;
    const only = members.length === 1 ? members[0]! : undefined;
    return only !== undefined && only.kind !== "union" ? only.text : `[${members.map((member) => member.text).join("")}]`;
  };

  const characterClass = (): string | null => {
    at += 1;
    const negated = pattern[at] === "^";
    if (negated) at += 1;
    const members: ClassMember[] = [];
    for (let first = true; ; first = false) {
      if (at >= pattern.length) return null;
      const rest = pattern.slice(at);
      if (rest.startsWith("]") && !first) {
        at += 1;
        break;
      }
      const posix = /^\[:(\^?)([a-z]+):\]/u.exec(rest);
      if (posix !== null) {
        // A POSIX class is the class of its expansion, read next as a nested one.
        if (posix[1] !== "" || POSIX_CLASSES[posix[2]!] === undefined) return null;
        pattern = `${pattern.slice(0, at)}[${POSIX_CLASSES[posix[2]!]!}]${pattern.slice(at + posix[0].length)}`;
      } else if (rest.startsWith("[")) {
        const nested = characterClass();
        if (nested === null) return null;
        members.push({ kind: "set", text: nested });
      } else if (rest.startsWith("~~")) {
        return null;
      } else if (rest.startsWith("&&") || rest.startsWith("--")) {
        operation = true;
        members.push({ kind: "operator", text: rest.slice(0, 2) as "&&" | "--" });
        at += 2;
      } else {
        const start = character();
        if (start === null) return null;
        if (start.kind === "character" && pattern[at] === "-" && pattern[at + 1] !== "-" && pattern[at + 1] !== "]" && at + 1 < pattern.length) {
          at += 1;
          const end = character();
          if (end === null || end.kind !== "character") return null;
          members.push({ kind: "union", text: `${start.text}-${end.text}` });
        } else {
          members.push(start);
        }
      }
    }
    const groups: ClassMember[][] = [[]];
    const operators: string[] = [];
    for (const member of members) {
      if (member.kind === "operator") {
        operators.push(member.text);
        groups.push([]);
      } else {
        groups[groups.length - 1]!.push(member);
      }
    }
    let expression: string | null;
    if (operators.length === 0) {
      expression = members.map((member) => member.text).join("");
    } else {
      expression = operand(groups[0]!);
      for (const [index, operator] of operators.entries()) {
        const right = operand(groups[index + 1]!);
        if (expression === null || right === null) return null;
        if (index > 0 && operator !== operators[index - 1]) expression = `[${expression}]`;
        expression += `${operator}${right}`;
      }
    }
    return expression === null ? null : `[${negated ? "^" : ""}${expression}]`;
  };

  let out = "";
  while (at < pattern.length) {
    const char = pattern[at]!;
    if (char === "\\") {
      const escape = /^\\(?:[pPxu]\{[^}]*\}|[\s\S])?/u.exec(pattern.slice(at))![0];
      out += escape.length === 2 && UNESCAPED_PUNCTUATION.includes(escape[1]!) ? escape[1]! : escape;
      at += escape.length;
    } else if (char === "[") {
      const text = characterClass();
      if (text === null) return null;
      out += text;
    } else {
      out += char;
      at += 1;
    }
  }
  return operation ? out : undefined;
}

/**
 * Rewrite a regex that the engine applies to one node's text so it can run over whole source text: `^` and `$`
 * outside character classes become identifier boundaries.
 */
export function boundaryAnchored(pattern: string): string {
  let out = "";
  let inClass = false;
  for (let index = 0; index < pattern.length; index += 1) {
    const char = pattern[index]!;
    if (char === "\\") {
      out += pattern.slice(index, index + 2);
      index += 1;
    } else if (inClass) {
      out += char;
      if (char === "]") inClass = false;
    } else if (char === "[") {
      inClass = true;
      out += char;
      if (pattern[index + 1] === "^") {
        out += "^";
        index += 1;
      }
    } else if (char === "^") out += "(?<![\\w$])";
    else if (char === "$") out += "(?![\\w$])";
    else out += char;
  }
  return out;
}

/**
 * The index after an empty match at `index`: one code point on, as String.prototype.matchAll steps. Stepping one code
 * unit lands inside a surrogate pair, where a unicode regex starts again at the pair and finds the same empty match.
 */
function afterEmptyMatch(text: string, index: number): number {
  return index + ((text.codePointAt(index) ?? 0) > 0xffff ? 2 : 1);
}

/** Whether every top-level alternative of a regex starts with `^` or `\A`, so on a node it matches at the node's start. */
export function anchoredAtStart(pattern: string): boolean {
  const body = pattern.replace(/^\(\?[a-zA-Z]+\)/u, "");
  const alternatives: string[] = [];
  let depth = 0;
  let inClass = false;
  let start = 0;
  for (let index = 0; index < body.length; index += 1) {
    const char = body[index]!;
    if (char === "\\") index += 1;
    else if (inClass) inClass = char !== "]";
    else if (char === "[") inClass = true;
    else if (char === "(") depth += 1;
    else if (char === ")") depth -= 1;
    else if (char === "|" && depth === 0) {
      alternatives.push(body.slice(start, index));
      start = index + 1;
    }
  }
  alternatives.push(body.slice(start));
  return alternatives.every((alternative) => alternative.startsWith("^") || alternative.startsWith("\\A"));
}

/** An identifier in source text; one right after a backslash is the tail of an escape sequence such as `\\n`. */
const IDENTIFIER = /(?<![\w$\\])[A-Za-z_$][\w$]*/gu;

/** The same identifier with every character changed and its shape kept, to test whether a regex depends on it. */
function mangle(word: string): string {
  return [...word].map((char) => {
    if (/[a-z]/u.test(char)) return char === "q" ? "z" : "q";
    if (/[A-Z]/u.test(char)) return char === "Q" ? "Z" : "Q";
    if (/[0-9]/u.test(char)) return char === "7" ? "8" : "7";
    return char;
  }).join("");
}

const LETTERS_AND_DIGITS = [..."ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"];
/** Longer identifiers keep the mangle test's answer: probing every position of a 100,000-letter one costs too much. */
const PIN_PROBE_LIMIT = 64;

/**
 * Whether a regex holds one particular letter or digit of a word: at some position, every other letter or digit (the
 * character's other case aside) stops the match. `[Aa]ddress`, `^f[o]o$` and `[0-9A-F]{3}1234567` hold some; a
 * class-shaped regex (`[0-9A-Fa-f]{7,}`) accepts other characters at every position, so it names no word.
 */
function pinsACharacter(word: string, matchesInPlace: (variant: string) => boolean): boolean {
  if (word.length > PIN_PROBE_LIMIT) return true;
  return [...word].some((char, index) => /[A-Za-z0-9]/u.test(char)
    && LETTERS_AND_DIGITS.every((other) => other.toLowerCase() === char.toLowerCase() || !matchesInPlace(word.slice(0, index) + other + word.slice(index + 1))));
}

/** Whether two words are one insertion, deletion or substitution apart. */
function oneEditApart(a: string, b: string): boolean {
  if (Math.abs(a.length - b.length) > 1 || a === b) return false;
  let start = 0;
  while (start < a.length && a[start] === b[start]) start += 1;
  let end = 0;
  while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) end += 1;
  return a.length - start - end <= 1 && b.length - start - end <= 1;
}

/** Every word one insertion, deletion or substitution from `word`, each made with a letter the word's shape allows. */
function oneEditVariants(word: string): string[] {
  const variants: string[] = [];
  for (let index = 0; index <= word.length; index += 1) {
    variants.push(`${word.slice(0, index)}q${word.slice(index)}`);
    if (index === word.length) continue;
    variants.push(word.slice(0, index) + word.slice(index + 1));
    variants.push(word.slice(0, index) + mangle(word[index]!) + word.slice(index + 1));
  }
  return variants;
}

/**
 * The unexplained identifiers in upstream's test text that a regex selects, and those of them it names. It selects an
 * identifier its matches touch whose text the match depends on (changing the identifier's letters stops the regex
 * matching there), so a regex that matches a whole `/// <reference path="...">` line selects none of the path's words.
 * Every selected identifier counts towards `limit`, past which the regex is a syntax pattern and the search stops. It
 * names a selected identifier unless it holds none of its letters or digits (a class-shaped regex such as
 * `[0-9A-Fa-f]{7,}`), or the identifier is one edit from an explained name whose every one-edit variant the regex
 * matches in its place: there the regex spells upstream's edit-distance check (Next.js's no-typos), not the test's typo.
 * Undefined when the gate cannot run the regex.
 */
function regexNames(regex: PolicyRegex, texts: readonly PreparedText[], unexplained: (word: string) => boolean, explainedNear: (word: string) => string[], limit: number): { selected: Set<string>; named: Set<string> } | undefined {
  const compiled = javascriptRegex(regex.wholeText ? regex.text : boundaryAnchored(regex.text), "g");
  if (compiled === undefined) return undefined;
  const overlaps = (text: string, from: number, to: number): boolean => {
    compiled.lastIndex = 0;
    for (let match = compiled.exec(text); match !== null; match = compiled.exec(text)) {
      const end = match.index + match[0].length;
      if (end === match.index) compiled.lastIndex = afterEmptyMatch(text, end);
      if (match.index < to && end > from) return true;
    }
    return false;
  };
  const selected = new Set<string>();
  const named = new Set<string>();
  // On a string node, letters at the node's start that run into a quote are its prefix (Python's `rb"`), not a name.
  const readsPrefix = !regex.wholeText && anchoredAtStart(regex.text);
  for (const { text, identifiers } of texts) {
    // An occurrence is tested once per text: the same word at the same offset of another text has other surroundings,
    // so whether the regex depends on it there is a separate question.
    const tested = new Set<number>();
    compiled.lastIndex = 0;
    const matches: { start: number; end: number }[] = [];
    for (let match = compiled.exec(text); match !== null; match = compiled.exec(text)) {
      if (match[0].length === 0) compiled.lastIndex = afterEmptyMatch(text, match.index);
      else matches.push({ start: match.index, end: match.index + match[0].length });
    }
    for (const { start, end } of matches) {
      // Only the identifiers the match overlaps: walking every identifier of the text for each match was quadratic in
      // the text, most of a cold python/ruff run.
      for (let at = firstEndingAfter(identifiers, start); at < identifiers.length && identifiers[at]!.start < end; at += 1) {
        const identifier = identifiers[at]!;
        if (selected.has(identifier.word) || !unexplained(identifier.word)) continue;
        if (readsPrefix && identifier.start === start && identifier.end < end && (text[identifier.end] === "'" || text[identifier.end] === '"')) continue;
        if (tested.has(identifier.start)) continue;
        tested.add(identifier.start);
        // Whole-text regexes may anchor at the start of the text; node regexes only need the surroundings, and always the
        // whole identifier (ESLint's tests hold 100,000-letter ones, which a fixed margin would cut).
        const from = regex.wholeText ? 0 : Math.max(0, Math.min(start - 512, identifier.start));
        const to = regex.wholeText ? text.length : Math.min(text.length, Math.max(end + 512, identifier.end));
        const matchesInPlace = (word: string) =>
          overlaps(text.slice(from, identifier.start) + word + text.slice(identifier.end, to), identifier.start - from, identifier.start - from + word.length);
        if (matchesInPlace(mangle(identifier.word))) continue;
        selected.add(identifier.word);
        if (pinsACharacter(identifier.word, matchesInPlace) && !explainedNear(identifier.word).some((name) => oneEditVariants(name).every(matchesInPlace))) named.add(identifier.word);
        if (selected.size > limit) return { selected, named };
      }
    }
  }
  return { selected, named };
}

/**
 * Whether a word a regex spells is the regex refusing a longer word rather than naming one: a run of that longer word's
 * letters written right before a negated class that excludes its next letter. Rust regex has no lookahead, so "not
 * followed by `pages`" is spelled through the prefixes of `pages` (`page(?:[^s]|$)`).
 */
function refusesLongerWord(pattern: string, word: string, longerWords: string[]): boolean {
  const refusals = pattern.matchAll(new RegExp(`(?<![\\w\\\\])${word}(?:\\(\\?:)?\\[\\^((?:\\\\.|[^\\]\\\\])*)\\]`, "gu"));
  for (const refusal of refusals) {
    const excluded = refusal[1]!.replace(/\\./gu, "");
    for (const longer of longerWords) {
      for (let at = longer.indexOf(word); at !== -1; at = longer.indexOf(word, at + 1)) {
        const next = longer[at + word.length];
        if (next !== undefined && excluded.includes(next)) return true;
      }
    }
  }
  return false;
}

/**
 * not-overfit: every name a rule hardcodes must be one upstream's code checks, in the rule's implementation or in the
 * shared helpers it delegates to, or syntax of the upstream language. A name that occurs in upstream's test files but
 * not in upstream's code is fixture-only: the rule passes the tests by matching the tests' own names. Every regex is
 * also run over upstream's test text and judged by the identifiers its matches touch, so `[Aa]ddress` or `^f[o]o$`
 * cannot hide a name; a regex the gate cannot run fails it. In a pattern's context, only the words of the node the
 * selector picks are matched; the rest is scaffolding.
 */
export function notOverfitGate(document: { clauses?: unknown }, inputs: NotOverfitInputs): NotOverfitResult {
  // An upstream regex names a test word when it selects it, as a rule's own regex does: it matches the word and stops
  // matching once the word's letters change. A regex that spells names (Bandit's `pas+wo?r?d|pwd|token|secrete?`)
  // names every word it selects; one that spells none is syntax-shaped (`^[A-Z]`, `[aeiou]`) and names nothing once
  // it selects more than the limit.
  const upstreamRegexNames = new Set<string>();
  for (const regex of inputs.implementationRegexes) {
    const plain = new RegExp(regex.source, regex.flags.replace(/[gy]/gu, ""));
    const selected = [...inputs.fixtures].filter((word) => plain.test(word) && !plain.test(mangle(word)));
    const spellsNames = regexWords(regex.source).some((word) => word.length >= 3);
    if (spellsNames || selected.length <= REGEX_NAME_LIMIT) for (const word of selected) upstreamRegexNames.add(word);
  }
  const explained = (word: string, commentText: boolean): LiteralClass | undefined => {
    if (inputs.keywords.has(word) || (commentText && inputs.commentVocabulary.has(word))) return "language";
    if (inputs.implementation.has(word) || upstreamRegexNames.has(word)) return "implementation";
    if (inputs.sharedImplementation.has(word)) return "shared-implementation";
    return undefined;
  };
  const explainedNames = [...inputs.implementation, ...inputs.sharedImplementation];
  const explainedNear = (word: string) => explainedNames.filter((name) => oneEditApart(word, name));

  const selections = new Map<string, Set<string> | undefined>();
  const selectedWords = (literal: PolicyLiteral): Set<string> | undefined => {
    if (literal.role !== "context" || literal.selector === undefined || literal.language === undefined) return undefined;
    const key = `${literal.language}\0${literal.selector}\0${literal.text}`;
    if (!selections.has(key)) {
      const text = inputs.selectedText(literal.text, literal.selector, literal.language);
      selections.set(key, text === undefined ? undefined : new Set(codeWords(text)));
    }
    return selections.get(key);
  };

  const classify = (literal: PolicyLiteral): LiteralClass => {
    const known = explained(literal.token, literal.commentText === true);
    if (known !== undefined) return known;
    // Without the selected node, every context word is judged: the conservative reading.
    if (literal.role === "context" && selectedWords(literal)?.has(literal.token) === false) return "scaffold";
    if (literal.role === "regex") {
      const longerWords = regexWords(affixedAlternations(literal.text)).filter((word) => word !== literal.token && explained(word, literal.commentText === true) !== undefined);
      if (refusesLongerWord(literal.text, literal.token, longerWords)) return "scaffold";
    }
    return inputs.fixtures.has(literal.token) ? "fixture-only" : "unexplained";
  };

  const extracted = policyLiterals(document);
  const literals: ClassifiedLiteral[] = extracted.map((literal) => ({ ...literal, class: classify(literal) }));

  const unchecked: string[] = [];
  const lowerExplained = new Set([...inputs.keywords, ...inputs.implementation, ...inputs.sharedImplementation].map((word) => word.toLowerCase()));
  const lowerComment = new Set([...inputs.commentVocabulary].map((word) => word.toLowerCase()));
  const fixtureTexts = prepareTexts(inputs.fixtureTexts);
  for (const regex of policyRegexes(document)) {
    // A case-insensitive regex matches upstream's own names in any case, so explain them in any case.
    const caseless = /\(\?[ims]*i[ims]*[:)]/u.test(regex.text);
    const isUnexplained = (word: string) => explained(word, regex.commentText) === undefined
      && !(caseless && (lowerExplained.has(word.toLowerCase()) || (regex.commentText && lowerComment.has(word.toLowerCase()))));
    const found = regexNames(regex, fixtureTexts, isUnexplained, explainedNear, REGEX_NAME_LIMIT);
    if (found === undefined) {
      unchecked.push(`${regex.pointer}: ${regex.text}`);
      continue;
    }
    if (found.selected.size === 0 || found.selected.size > REGEX_NAME_LIMIT) continue;
    for (const word of [...found.named].sort()) {
      if (literals.some((entry) => entry.pointer === regex.pointer && entry.token === word)) continue;
      literals.push({ token: word, pointer: regex.pointer, role: "regex", text: regex.text, commentText: regex.commentText, class: "fixture-only" });
    }
  }
  for (const literal of literals) {
    if (literal.role === "regex" && literal.class === "fixture-only" && !inputs.fixtures.has(literal.token)) literal.class = "unexplained";
  }

  const flagged = [...new Set(literals.filter((literal) => literal.class === "fixture-only").map((literal) => literal.token))].sort();
  return {
    checks: [
      {
        id: "no-fixture-only-names",
        ok: flagged.length === 0,
        detail: flagged.length === 0
          ? `${new Set(literals.map((literal) => literal.token)).size} hardcoded word(s); none occurs only in upstream's tests`
          : `matches ${flagged.length} name(s) that occur in upstream's tests but not in upstream's code: ${flagged.join(", ")}`,
      },
      {
        id: "regexes-judged",
        ok: unchecked.length === 0,
        detail: unchecked.length === 0 ? "every regex was run over upstream's tests" : `cannot judge ${unchecked.length} regex(es) written in syntax the gate cannot run, e.g. ${unchecked[0]}`,
      },
    ],
    literals,
    unchecked,
  };
}
