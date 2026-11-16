/**
 * JSON with comments and trailing commas, as ESLint's, oxlint's and Biome's JSON config files allow. The comments and
 * trailing commas are blanked out and the rest goes to `JSON.parse`, so only real JSON is read: anything else is
 * refused with `LintConfigFormatError` rather than guessed at.
 */
import { LintConfigFormatError } from "./lint-config-formats.js";

/**
 * `text` as JSONC: JSON plus `//` and `/* *\/` comments and a trailing comma before `]` or `}`, a leading byte order
 * mark allowed. Throws `LintConfigFormatError` on anything else (YAML included).
 */
export function parseJsonc(text: string): unknown {
  const json = new JsoncScanner(text).json();
  try {
    return JSON.parse(json);
  } catch (error) {
    throw new LintConfigFormatError(`not JSON: ${(error as Error).message}`);
  }
}

/** Turns JSONC into the JSON `JSON.parse` reads: strings copied whole, each comment blanked, each trailing comma dropped. */
class JsoncScanner {
  readonly #text: string;
  readonly #out: string[] = [];
  /** Where in the output the last comma is, while only blanks and comments follow it. */
  #pendingComma: number | undefined;
  #index: number;

  constructor(text: string) {
    this.#text = text;
    this.#index = text.startsWith("\uFEFF") ? 1 : 0;
  }

  json(): string {
    while (this.#index < this.#text.length) this.#token();
    return this.#out.join("");
  }

  #token(): void {
    const char = this.#text[this.#index]!;
    if (char === "\"") this.#string();
    else if (this.#text.startsWith("//", this.#index)) this.#lineComment();
    else if (this.#text.startsWith("/*", this.#index)) this.#blockComment();
    else this.#char(char);
  }

  #string(): void {
    const end = jsonStringEnd(this.#text, this.#index);
    this.#out.push(this.#text.slice(this.#index, end));
    this.#pendingComma = undefined;
    this.#index = end;
  }

  #lineComment(): void {
    const end = this.#text.indexOf("\n", this.#index);
    this.#index = end < 0 ? this.#text.length : end;
  }

  #blockComment(): void {
    const end = this.#text.indexOf("*/", this.#index + 2);
    if (end < 0) throw new LintConfigFormatError("a /* comment is not closed");
    this.#out.push(" ");
    this.#index = end + 2;
  }

  /** Any other character, copied; a `]` or `}` blanks the comma before it, if only blanks and comments lie between. */
  #char(char: string): void {
    if ((char === "]" || char === "}") && this.#pendingComma !== undefined) this.#out[this.#pendingComma] = " ";
    if (char === ",") this.#pendingComma = this.#out.length;
    else if (!/\s/u.test(char)) this.#pendingComma = undefined;
    this.#out.push(char);
    this.#index += 1;
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
