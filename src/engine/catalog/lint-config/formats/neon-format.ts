/**
 * NEON, the format of PHPStan's config files, read as nette/neon 3.3.4 reads it (the version PHPStan 2.2.16's
 * composer.lock pins; `src/Neon/Lexer.php`, `Parser.php`, `Node/LiteralNode.php`, `Node/StringNode.php`): block
 * mappings and lists by indentation, inline `[...]` and `{...}` arrays over several lines, `key: value` and
 * `key=value`, quoted and multi-line strings, comments, and literals as booleans, `null`, numbers or strings. An entity
 * (`Name(...)`, as `services` use) is read only as one: Rerail does not interpret it. What nette/neon would refuse,
 * this throws `LintConfigFormatError` for, with the line.
 */
import { LintConfigFormatError } from "./lint-config-formats.js";

/** A NEON value; an array is both list and mapping, as a PHP array is. */
export type NeonValue = string | number | boolean | null | NeonArray | NeonEntity;

/** An array: items with a key (`key: value`) or without one (`- value`, `[value]`), in order. */
export interface NeonArray {
  readonly kind: "array";
  readonly items: readonly NeonItem[];
}

export interface NeonItem {
  readonly key: string | undefined;
  readonly value: NeonValue;
}

/** `Name(...)`, or a chain of them: read past, not interpreted. */
export interface NeonEntity {
  readonly kind: "entity";
}

/** `text` as NEON; throws `LintConfigFormatError` where nette/neon would fail. */
export function parseNeon(text: string): NeonValue {
  return new NeonParser(tokenize(text.replaceAll("\r", ""))).parse();
}

/** Whether `value` is an array. */
export const isNeonArray = (value: NeonValue | undefined): value is NeonArray => typeof value === "object" && value !== null && value.kind === "array";

/** The value at `key` of array `value`; undefined when it is not an array or has no such key. */
export function neonEntry(value: NeonValue | undefined, key: string): NeonValue | undefined {
  return isNeonArray(value) ? value.items.find(item => item.key === key)?.value : undefined;
}

/** A token: a string, a literal, a newline run, whitespace, a comment, or one punctuation character as itself. */
interface Token {
  readonly type: "string" | "literal" | "newline" | "whitespace" | "comment" | string;
  readonly value: string;
  readonly offset: number;
}

/**
 * Lexer.php's patterns, in its order (`~Amixu`): JavaScript has no possessive quantifiers, but nothing follows these
 * groups inside a pattern, so the greedy ones match the same text.
 */
const TOKEN_PATTERNS: readonly (readonly [Token["type"], RegExp])[] = [
  ["string", new RegExp([
    String.raw`'''\n(?:(?:[^\n]|\n(?![\t ]*'''))*\n)?[\t ]*'''`,
    String.raw`"""\n(?:(?:[^\n]|\n(?![\t ]*"""))*\n)?[\t ]*"""`,
    String.raw`'(?:''|[^'\n])*'`,
    String.raw`"(?:\\.|[^"\\\n])*"`,
  ].join("|"), "uy")],
  ["literal", new RegExp(
    String.raw`(?:[^#"',:=[\]{}()\n\t \x60-]|(?<!["'])[:-][^"',=[\]{}()\n\t ])`
    + String.raw`(?:[^,:=\]})(\n\t ]+|:(?![\n\t ,\]})]|$)|[ \t]+[^#,:=\]})(\n\t ])*`,
    "muy",
  )],
  ["char", /[,:=[\]{}()-]/uy],
  ["comment", /#.*/uy],
  ["newline", /\n+/uy],
  ["whitespace", /[\t ]+/uy],
];

function tokenAt(text: string, offset: number): Token | undefined {
  for (const [type, pattern] of TOKEN_PATTERNS) {
    pattern.lastIndex = offset;
    const match = pattern.exec(text);
    if (match !== null) return { type: type === "char" ? match[0] : type, value: match[0], offset };
  }
  return undefined;
}

function tokenize(text: string): { text: string; tokens: Token[] } {
  const tokens: Token[] = [];
  let offset = 0;
  while (offset < text.length) {
    const token = tokenAt(text, offset);
    if (token === undefined) throw neonError(text, offset, `unexpected '${text.slice(offset, offset + 20)}'`);
    tokens.push(token);
    offset += token.value.length;
  }
  return { text, tokens };
}

function neonError(text: string, offset: number, why: string): LintConfigFormatError {
  return new LintConfigFormatError(`not NEON: ${why} on line ${text.slice(0, offset).split("\n").length}`);
}

/** LiteralNode::SimpleTypes: the literals that are booleans or null, except as keys. */
const SIMPLE_TYPES: ReadonlyMap<string, boolean | null> = new Map([
  ...["true", "True", "TRUE", "yes", "Yes", "YES", "on", "On", "ON"].map((word): [string, boolean] => [word, true]),
  ...["false", "False", "FALSE", "no", "No", "NO", "off", "Off", "OFF"].map((word): [string, boolean] => [word, false]),
  ...["null", "Null", "NULL"].map((word): [string, null] => [word, null]),
]);
/** PHP's `is_numeric`, as LiteralNode::parse uses it, with hexadecimal, octal and binary literals after it. */
const NUMERIC = /^[ \t\n\r\v\f]*[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?[ \t\n\r\v\f]*$/u;
const RADIX: readonly (readonly [RegExp, number])[] = [[/^0x([0-9a-fA-F]+)$/u, 16], [/^0o([0-7]+)$/u, 8], [/^0b([01]+)$/u, 2]];

/** A literal's value: LiteralNode::parse, dates kept as their text. */
function literalValue(text: string, isKey: boolean): string | number | boolean | null {
  const simple = SIMPLE_TYPES.get(text);
  if (!isKey && simple !== undefined) return simple;
  if (NUMERIC.test(text)) return Number(text);
  for (const [pattern, radix] of RADIX) {
    const digits = pattern.exec(text)?.[1];
    if (digits !== undefined) return Number.parseInt(digits, radix);
  }
  return text;
}

/** StringNode::EscapeSequences. */
const ESCAPES: Readonly<Record<string, string>> = { t: "\t", n: "\n", r: "\r", f: "\f", b: "\b", "\"": "\"", "\\": "\\", "/": "/", _: "\u00A0" };

/** A string token's value: StringNode::parse. Undefined for an escape it refuses. */
function stringValue(token: string): string | undefined {
  const multiline = /^...\n+([\t ]*)/u.exec(token);
  const body = multiline === null
    ? token.slice(1, -1)
    : token.slice(3, -3).replaceAll(`\n${multiline[1]!}`, "\n").replace(/^\n|\n[\t ]*$/gu, "");
  if (token.startsWith("'")) return multiline === null ? body.replaceAll("''", "'") : body;
  let refused = false;
  const value = body.replace(/\\(?:ud[89ab][0-9a-f]{2}\\ud[c-f][0-9a-f]{2}|u[0-9a-f]{4}|x[0-9a-f]{2}|.)/giu, sequence => {
    const escaped = escapeText(sequence);
    if (escaped === undefined) refused = true;
    return escaped ?? "";
  });
  return refused ? undefined : value;
}

function escapeText(sequence: string): string | undefined {
  const simple = ESCAPES[sequence[1]!];
  if (simple !== undefined) return simple;
  if (/^\\u/iu.test(sequence) && sequence.length >= 6) return JSON.parse(`"${sequence.toLowerCase()}"`) as string;
  if (/^\\x/iu.test(sequence) && sequence.length === 4) return String.fromCharCode(Number.parseInt(sequence.slice(2), 16));
  return undefined;
}

const array = (items: readonly NeonItem[]): NeonArray => ({ kind: "array", items });
const CLOSING: Readonly<Record<string, string>> = { "[": "]", "{": "}", "(": ")" };

/** What a block item starts with: a dash, a key, or a value that is the whole block. */
type ItemHead = { readonly done: false; readonly key: string | undefined } | { readonly done: true; readonly value: NeonValue };

/** Parser.php over the tokens, one method per method of it. */
class NeonParser {
  #pos = 0;
  readonly #text: string;
  readonly #tokens: readonly Token[];

  constructor({ text, tokens }: { text: string; tokens: Token[] }) {
    this.#text = text;
    this.#tokens = tokens;
  }

  parse(): NeonValue {
    this.skipNewlines();
    const value = this.parseBlock(this.indentation());
    this.skipNewlines();
    if (this.isNext()) this.fail();
    return value;
  }

  private parseBlock(indent: string, onlyBullets = false): NeonValue {
    const items: NeonItem[] = [];
    const keys = new Set<string>();
    for (;;) {
      const head = this.itemHead(items.length > 0, onlyBullets, keys);
      if (head.done) return items.length > 0 ? array(items) : head.value;
      const { value, closed } = this.itemValue(indent, head.key);
      items.push({ key: head.key, value });
      if (closed || !this.blockContinues(indent)) return array(items);
    }
  }

  private itemHead(hasItems: boolean, onlyBullets: boolean, keys: Set<string>): ItemHead {
    if (this.consume("-")) return { done: false, key: undefined };
    if (!this.isNext() || onlyBullets) return { done: true, value: null };
    const start = this.#pos;
    const value = this.parseValue();
    if (this.consume(":", "=")) return { done: false, key: this.checkKey(value, keys, start) };
    if (hasItems) this.fail();
    return { done: true, value };
  }

  /** The value after a dash or a key: a nested block, an inline value, or null. */
  private itemValue(indent: string, key: string | undefined): { value: NeonValue; closed?: boolean } {
    if (this.consume("newline")) return this.valueOnNextLines(indent, key);
    if (key === undefined) return { value: this.parseBlock(`${indent}  `) };
    if (!this.isNext()) return { value: null };
    const value = this.parseValue();
    if (this.isNext() && !this.isNext("newline")) this.fail();
    return { value };
  }

  /** The value of an item whose line ends after its dash or key: a deeper block, a dash sub-block of a key, or null. */
  private valueOnNextLines(indent: string, key: string | undefined): { value: NeonValue; closed?: boolean } {
    this.skipNewlines();
    const next = this.indentation();
    this.checkIndentation(next, indent);
    if (next.length > indent.length) return { value: this.parseBlock(next) };
    if (next.length < indent.length) return { value: null, closed: true };
    return { value: key !== undefined && this.isNext("-") ? this.parseBlock(indent, true) : null };
  }

  /** Whether the next item is in the same block, past blank lines; a deeper one is bad indentation. */
  private blockContinues(indent: string): boolean {
    this.skipNewlines();
    if (!this.isNext()) return false;
    const next = this.indentation();
    this.checkIndentation(next, indent);
    if (next.length > indent.length) this.fail("bad indentation");
    return next.length === indent.length;
  }

  private checkIndentation(next: string, indent: string): void {
    const shared = Math.min(next.length, indent.length);
    if (next.slice(0, shared) !== indent.slice(0, shared)) this.fail("invalid combination of tabs and spaces");
  }

  private parseValue(): NeonValue {
    const token = this.consume("string", "literal");
    let value: NeonValue;
    if (token?.type === "string") value = stringValue(token.value) ?? this.fail("invalid escaping sequence");
    else if (token !== undefined) value = literalValue(token.value, this.isNext(":", "="));
    else if (this.isNext("[", "(", "{")) value = this.parseBraces();
    else this.fail();
    return this.parseEntity(value);
  }

  /** An entity after `value`, read past: `Name(...)`, and any chain of literals and attributes after it. */
  private parseEntity(value: NeonValue): NeonValue {
    if (!this.isNext("(")) return value;
    this.parseBraces();
    while (this.consume("literal") !== undefined) {
      if (!this.isNext("(")) break;
      this.parseBraces();
    }
    return { kind: "entity" };
  }

  private parseBraces(): NeonArray {
    // Only called at a `[`, `(` or `{`.
    const open = this.consume()!.value;
    const end = CLOSING[open]!;
    const items: NeonItem[] = [];
    const keys = new Set<string>();
    for (;;) {
      this.skipNewlines();
      if (this.consume(end)) return array(items);
      items.push(this.inlineItem(end, keys));
      if (this.consume(",", "newline")) continue;
      this.skipNewlines();
      if (!this.isNext(end)) this.fail();
    }
  }

  private inlineItem(end: string, keys: Set<string>): NeonItem {
    const start = this.#pos;
    const value = this.parseValue();
    if (!this.consume(":", "=")) return { key: undefined, value };
    const key = this.checkKey(value, keys, start);
    return { key, value: this.isNext("newline", ",", end) ? null : this.parseValue() };
  }

  /**
   * A key's text: a string or a number literal, once per array. A literal before `:` or `=` is read as a key, never as
   * a boolean or null (LiteralNode::parse's `$isKey`), so only an array or entity is no key.
   */
  private checkKey(value: NeonValue, keys: Set<string>, start: number): string {
    if (typeof value === "object") this.fail("unacceptable key", start);
    const key = String(value);
    if (keys.has(key)) this.fail(`duplicated key '${key}'`, start);
    keys.add(key);
    return key;
  }

  private skipNewlines(): void {
    while (this.consume("newline") !== undefined);
  }

  /** TokenStream::getIndentation: the whitespace that starts the current line, when the next token starts it. */
  private indentation(): string {
    const before = this.#tokens[this.#pos - 2];
    const last = this.#tokens[this.#pos - 1];
    return (before === undefined || before.type === "newline") && last?.type === "whitespace" ? last.value : "";
  }

  /** TokenStream::isNext: past whitespace and comments, whether the next token is of one of `types` (any, when none). */
  private isNext(...types: string[]): boolean {
    while (this.#tokens[this.#pos]?.type === "whitespace" || this.#tokens[this.#pos]?.type === "comment") this.#pos += 1;
    const next = this.#tokens[this.#pos];
    return next !== undefined && (types.length === 0 || types.includes(next.type));
  }

  private consume(...types: string[]): Token | undefined {
    return this.isNext(...types) ? this.#tokens[this.#pos++] : undefined;
  }

  private fail(why?: string, at: number = this.#pos): never {
    const token = this.#tokens[at];
    const offset = token?.offset ?? this.#text.length;
    throw neonError(this.#text, offset, why ?? (token === undefined ? "unexpected end" : `unexpected '${token.value.slice(0, 40).replaceAll("\n", "<new line>")}'`));
  }
}
