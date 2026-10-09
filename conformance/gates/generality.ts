/**
 * Check 8 (generality): the rule validation ladder's V1 Gate 3 (nera-rule-validation `src/validation/gates/generality.ts`).
 * Its inputs come from the tool's recorded source facts (`upstream/<tool>/source.json`) and the fixture files copied
 * beside them. V1 probed the policy's regexes over every upstream test text (test code and expected outputs); here
 * they are probed over the copies and over every word of that text (`testWords`) on its own, since source.json
 * cannot hold the texts themselves (Ruff's fixtures are 6.9 MB). The words matter: a regex that selects more than
 * `REGEX_NAME_LIMIT` test identifiers is a syntax pattern, and the copies alone hold too few identifiers to tell
 * (Ruff F601's `[jJ]\)*$` selects only `j` and `obj` in them). A word of a test's configuration or messages
 * (`configurationWords`) is still fixture-only when a document hardcodes it, but no regex is run over it, as V1 ran
 * none. What the words lose is context: a regex that needs the text around a word (`myHook\.use`) selects it only
 * where the copies hold that text.
 */
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { compareDeterministicStrings } from "../../src/stable-json.js";
import type { UpstreamSource } from "../languages/pack.js";
import { LITERAL_CLASSES } from "../validation-report.js";
import type { GateCheck } from "./check.js";
import { affixedAlternations, codeWords, policyLiterals, policyRegexes, regexWords, type PolicyLiteral, type PolicyRegex } from "./policy-literals.js";

export type LiteralClass = (typeof LITERAL_CLASSES)[number];

export interface ClassifiedLiteral extends PolicyLiteral {
  class: LiteralClass;
}

/** What upstream says about names, and how to find the node a pattern context selects. */
export interface GeneralityInputs {
  /** Keywords of the upstream language: syntax in every role. */
  keywords: ReadonlySet<string>;
  /** Words the language gives meaning to inside comments (documentation tags, directives): syntax only in regexes. */
  commentVocabulary: ReadonlySet<string>;
  /** Names the rule's own upstream implementation checks. */
  implementation: ReadonlySet<string>;
  /** Names the shared rule helpers it delegates to check. */
  sharedImplementation: ReadonlySet<string>;
  /** Regexes upstream's implementation matches names with; the few test words each selects are names upstream checks. */
  implementationRegexes: readonly RegExp[];
  /** Every word in upstream's test files and test configurations. */
  fixtures: ReadonlySet<string>;
  /** The test texts the policy's regexes are run over. */
  fixtureTexts: readonly string[];
  /** The source text of the node a pattern context selects, or undefined when the engine cannot find it. */
  selectedText(context: string, selector: string, language: string): string | undefined;
}

export interface GeneralityResult {
  /** `generality:names` and `generality:regexes`. */
  checks: GateCheck[];
  literals: ClassifiedLiteral[];
  /** Regexes the gate could not run as JavaScript regular expressions; the gate fails rather than pass them unjudged. */
  unchecked: string[];
}

/**
 * Check 8's inputs for one port, from its tool's source facts: undefined when `source.json` records no such upstream
 * rule, which fails `provenance:upstream-rule`.
 */
export function inputsFromSource(
  source: UpstreamSource,
  rule: string,
  fixtureTexts: readonly string[],
  selectedText: GeneralityInputs["selectedText"],
): GeneralityInputs | undefined {
  const facts = source.rules[rule];
  if (facts === undefined) return undefined;
  return {
    keywords: new Set(source.keywords),
    commentVocabulary: new Set(source.commentVocabulary),
    implementation: new Set(facts.names.implementation),
    sharedImplementation: new Set([...source.sharedNames, ...facts.names.shared]),
    implementationRegexes: facts.regexes.map(regex => new RegExp(regex.source, regex.flags)),
    fixtures: new Set([...source.testWords, ...source.configurationWords]),
    fixtureTexts: [...fixtureTexts, ...source.testWords],
    selectedText,
  };
}

/** The text of every file under a tool's copied fixtures (`upstream/<tool>/fixtures/`), in path order. */
export function readFixtureTexts(directory: string): string[] {
  return readdirSync(directory, { recursive: true, withFileTypes: true })
    .filter(entry => entry.isFile())
    .map(entry => join(entry.parentPath, entry.name))
    .sort(compareDeterministicStrings)
    .map(path => readFileSync(path, "utf8"));
}

/** Check 8's working, as the pack validation report records it. */
export function generalityReport(result: GeneralityResult): {
  words: Record<LiteralClass, number>;
  flagged: { token: string; pointer: string; role: PolicyLiteral["role"]; text: string }[];
  uncheckedRegexes: string[];
} {
  const words = Object.fromEntries(LITERAL_CLASSES.map(name => [name, 0])) as Record<LiteralClass, number>;
  for (const literal of result.literals) words[literal.class] += 1;
  return {
    words,
    flagged: result.literals.filter(literal => literal.class === "fixture-only").map(({ token, pointer, role, text }) => ({ token, pointer, role, text })),
    uncheckedRegexes: result.unchecked,
  };
}

/** A regex that touches more distinct unexplained test identifiers than this is a syntax pattern, not a list of names. */
export const REGEX_NAME_LIMIT = 10;

const POSIX_CLASSES: Record<string, string> = {
  alnum: "A-Za-z0-9",
  alpha: "A-Za-z",
  digit: "0-9",
  lower: "a-z",
  upper: "A-Z",
  space: "\\s",
  word: "\\w",
  xdigit: "0-9A-Fa-f",
  punct: "!-\\/:-@\\[-`{-~",
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
  if (/\(\?[a-zA-Z-]+\)/u.test(unescaped) || [...unescaped.matchAll(/\(\?([a-zA-Z-]+):/gu)].some(group => /[^ims-]/u.test(group[1]!))) return undefined;
  const source = body
    .replaceAll("(?P<", "(?<")
    .replaceAll("\\A", "^")
    .replaceAll("\\z", "$")
    .replace(/\\x\{([0-9A-Fa-f]+)\}/gu, "\\u{$1}")
    // Rust lets any punctuation be escaped; JavaScript's unicode mode accepts escapes only for syntax characters. Escapes
    // are read in pairs, so the quote after an escaped backslash (`\\'`) stays a quote.
    .replace(/\\(.)/gsu, (whole, char: string) => (`!"#%&',:;<=>@_\`~ `.includes(char) ? char : whole))
    .replace(/\[:(\^?)([a-z]+):\]/gu, (whole, negated: string, name: string) => (negated === "" && POSIX_CLASSES[name] !== undefined ? POSIX_CLASSES[name] : whole));
  // Rust's class intersection and difference (`[\s&&[^\xA0]]`) are JavaScript's in unicodeSets mode, which reads the
  // rest of these patterns as unicode mode does and wants a range operand bracketed (`[[a-z]--[aeiou]]`); Rust's
  // symmetric difference (`~~`) JavaScript lacks.
  const setOperation = /(?:\[(?:\\.|[^\]\\])*|\])(&&|--|~~)/u.exec(source)?.[1];
  if (setOperation === "~~") return undefined;
  try {
    return new RegExp(source, `${[...new Set(`${flags}${extraFlags}`)].join("")}${setOperation === undefined ? "u" : "v"}`);
  } catch {
    return undefined;
  }
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
  return alternatives.every(alternative => alternative.startsWith("^") || alternative.startsWith("\\A"));
}

/** An identifier in source text; one right after a backslash is the tail of an escape sequence such as `\\n`. */
const IDENTIFIER = /(?<![\w$\\])[A-Za-z_$][\w$]*/gu;

/** The same identifier with every character changed and its shape kept, to test whether a regex depends on it. */
function mangle(word: string): string {
  return [...word]
    .map(char => {
      if (/[a-z]/u.test(char)) return char === "q" ? "z" : "q";
      if (/[A-Z]/u.test(char)) return char === "Q" ? "Z" : "Q";
      if (/[0-9]/u.test(char)) return char === "7" ? "8" : "7";
      return char;
    })
    .join("");
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
  return [...word].some(
    (char, index) =>
      /[A-Za-z0-9]/u.test(char) &&
      LETTERS_AND_DIGITS.every(other => other.toLowerCase() === char.toLowerCase() || !matchesInPlace(word.slice(0, index) + other + word.slice(index + 1))),
  );
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
function regexNames(
  regex: PolicyRegex,
  texts: readonly string[],
  unexplained: (word: string) => boolean,
  explainedNear: (word: string) => string[],
  limit: number,
): { selected: Set<string>; named: Set<string> } | undefined {
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
  const tested = new Set<string>();
  // On a string node, letters at the node's start that run into a quote are its prefix (Python's `rb"`), not a name.
  const readsPrefix = !regex.wholeText && anchoredAtStart(regex.text);
  for (const text of texts) {
    const identifiers = [...text.matchAll(IDENTIFIER)].map(match => ({ start: match.index, end: match.index + match[0].length, word: match[0] }));
    compiled.lastIndex = 0;
    const matches: { start: number; end: number }[] = [];
    for (let match = compiled.exec(text); match !== null; match = compiled.exec(text)) {
      if (match[0].length === 0) compiled.lastIndex = afterEmptyMatch(text, match.index);
      else matches.push({ start: match.index, end: match.index + match[0].length });
    }
    for (const { start, end } of matches) {
      for (const identifier of identifiers) {
        if (identifier.start >= end || identifier.end <= start || selected.has(identifier.word) || !unexplained(identifier.word)) continue;
        if (readsPrefix && identifier.start === start && identifier.end < end && (text[identifier.end] === "'" || text[identifier.end] === '"')) continue;
        const occurrence = `${identifier.word}\0${identifier.start}`;
        if (tested.has(occurrence)) continue;
        tested.add(occurrence);
        // Whole-text regexes may anchor at the start of the text; node regexes only need the surroundings, and always the
        // whole identifier (ESLint's tests hold 100,000-letter ones, which a fixed margin would cut).
        const from = regex.wholeText ? 0 : Math.max(0, Math.min(start - 512, identifier.start));
        const to = regex.wholeText ? text.length : Math.min(text.length, Math.max(end + 512, identifier.end));
        const matchesInPlace = (word: string): boolean =>
          overlaps(text.slice(from, identifier.start) + word + text.slice(identifier.end, to), identifier.start - from, identifier.start - from + word.length);
        if (matchesInPlace(mangle(identifier.word))) continue;
        selected.add(identifier.word);
        if (pinsACharacter(identifier.word, matchesInPlace) && !explainedNear(identifier.word).some(name => oneEditVariants(name).every(matchesInPlace))) named.add(identifier.word);
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
 * Check 8: every name a rule hardcodes must be one upstream's code checks, in the rule's implementation or in the
 * shared helpers it delegates to, or syntax of the upstream language. A name that occurs in upstream's test files but
 * not in upstream's code is fixture-only: the rule passes the tests by matching the tests' own names. Every regex is
 * also run over the test texts and judged by the identifiers its matches touch, so `[Aa]ddress` or `^f[o]o$` cannot
 * hide a name; a regex the gate cannot run fails it. In a pattern's context, only the words of the node the selector
 * picks are matched; the rest is scaffolding. `document` is the rule as the catalog stores it (`codeRuleSchema`'s
 * output), with its options resolved: the branch the default configuration takes is what is judged.
 */
export function generalityGate(document: { clauses?: unknown }, inputs: GeneralityInputs): GeneralityResult {
  // An upstream regex names a test word when it selects it, as a rule's own regex does: it matches the word and stops
  // matching once the word's letters change. A regex that spells names (Bandit's `pas+wo?r?d|pwd|token|secrete?`)
  // names every word it selects; one that spells none is syntax-shaped (`^[A-Z]`, `[aeiou]`) and names nothing once
  // it selects more than the limit.
  const upstreamRegexNames = new Set<string>();
  for (const regex of inputs.implementationRegexes) {
    const plain = new RegExp(regex.source, regex.flags.replace(/[gy]/gu, ""));
    const selected = [...inputs.fixtures].filter(word => plain.test(word) && !plain.test(mangle(word)));
    const spellsNames = regexWords(regex.source).some(word => word.length >= 3);
    if (spellsNames || selected.length <= REGEX_NAME_LIMIT) for (const word of selected) upstreamRegexNames.add(word);
  }
  const explained = (word: string, commentText: boolean): LiteralClass | undefined => {
    if (inputs.keywords.has(word) || (commentText && inputs.commentVocabulary.has(word))) return "language";
    if (inputs.implementation.has(word) || upstreamRegexNames.has(word)) return "implementation";
    if (inputs.sharedImplementation.has(word)) return "shared-implementation";
    return undefined;
  };
  const explainedNames = [...inputs.implementation, ...inputs.sharedImplementation];
  const explainedNear = (word: string): string[] => explainedNames.filter(name => oneEditApart(word, name));

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
      const longerWords = regexWords(affixedAlternations(literal.text)).filter(word => word !== literal.token && explained(word, literal.commentText === true) !== undefined);
      if (refusesLongerWord(literal.text, literal.token, longerWords)) return "scaffold";
    }
    return inputs.fixtures.has(literal.token) ? "fixture-only" : "unexplained";
  };

  const extracted = policyLiterals(document);
  const literals: ClassifiedLiteral[] = extracted.map(literal => ({ ...literal, class: classify(literal) }));

  const unchecked: string[] = [];
  const lowerExplained = new Set([...inputs.keywords, ...inputs.implementation, ...inputs.sharedImplementation].map(word => word.toLowerCase()));
  const lowerComment = new Set([...inputs.commentVocabulary].map(word => word.toLowerCase()));
  for (const regex of policyRegexes(document)) {
    // A case-insensitive regex matches upstream's own names in any case, so explain them in any case.
    const caseless = /\(\?[ims]*i[ims]*[:)]/u.test(regex.text);
    const isUnexplained = (word: string): boolean =>
      explained(word, regex.commentText) === undefined && !(caseless && (lowerExplained.has(word.toLowerCase()) || (regex.commentText && lowerComment.has(word.toLowerCase()))));
    const found = regexNames(regex, inputs.fixtureTexts, isUnexplained, explainedNear, REGEX_NAME_LIMIT);
    if (found === undefined) {
      unchecked.push(`${regex.pointer}: ${regex.text}`);
      continue;
    }
    if (found.selected.size === 0 || found.selected.size > REGEX_NAME_LIMIT) continue;
    for (const word of [...found.named].sort(compareDeterministicStrings)) {
      if (literals.some(entry => entry.pointer === regex.pointer && entry.token === word)) continue;
      literals.push({ token: word, pointer: regex.pointer, role: "regex", text: regex.text, commentText: regex.commentText, class: "fixture-only" });
    }
  }
  for (const literal of literals) {
    if (literal.role === "regex" && literal.class === "fixture-only" && !inputs.fixtures.has(literal.token)) literal.class = "unexplained";
  }

  const flagged = [...new Set(literals.filter(literal => literal.class === "fixture-only").map(literal => literal.token))].sort(compareDeterministicStrings);
  return {
    checks: [
      {
        id: "generality:names",
        ok: flagged.length === 0,
        detail:
          flagged.length === 0
            ? `${new Set(literals.map(literal => literal.token)).size} hardcoded word(s); none occurs only in upstream's tests`
            : `matches ${flagged.length} name(s) that occur in upstream's tests but not in upstream's code: ${flagged.join(", ")}`,
      },
      {
        id: "generality:regexes",
        ok: unchecked.length === 0,
        detail: unchecked.length === 0 ? "every regex was run over upstream's tests" : `cannot judge ${unchecked.length} regex(es) written in syntax the gate cannot run, e.g. ${unchecked[0]}`,
      },
    ],
    literals,
    unchecked,
  };
}
