/**
 * `JSON.parse` keeps the last of repeated object keys, so `{"kind":"a","kind":"b"}` silently becomes
 * `{"kind":"b"}` and nothing downstream (the schema, or the kernel's own repeated-key check) ever sees
 * the first. A reviver cannot tell either, so this is a minimal scanner over the raw text.
 */

/** One repeated key: the object's JSON Pointer and the key, compared after unescaping. */
export interface DuplicateJsonKey {
  pointer: string;
  key: string;
}

function escapePointerToken(token: string): string {
  return token.replaceAll("~", "~0").replaceAll("/", "~1");
}

/** Describe a repeated key the way the kernel does: "duplicate key `kind` at /checks/0/forbid". */
export function describeDuplicateJsonKey({ pointer, key }: DuplicateJsonKey): string {
  return `duplicate key \`${key}\` at ${pointer === "" ? "/" : pointer}`;
}

class Scanner {
  readonly #text: string;
  #index = 0;
  readonly duplicates: DuplicateJsonKey[] = [];
  #depth = 0;
  static readonly MAX_NESTING = 1_000;

  constructor(text: string) {
    this.#text = text;
  }

  scan(): void {
    this.#value("");
    this.#whitespace();
    if (this.#index !== this.#text.length) throw new SyntaxError(`Unexpected text after JSON at position ${this.#index}`);
  }

  #whitespace(): void {
    while (this.#index < this.#text.length && " \t\n\r".includes(this.#text[this.#index]!)) this.#index += 1;
  }

  #expect(character: string): void {
    if (this.#text[this.#index] !== character) {
      throw new SyntaxError(`Expected '${character}' at position ${this.#index} of JSON`);
    }
    this.#index += 1;
  }

  #enterContainer(): void {
    if (this.#depth >= Scanner.MAX_NESTING) throw new RangeError("JSON nesting exceeds duplicate-key validation limit");
    this.#depth += 1;
  }

  #leaveContainer(): void {
    this.#depth -= 1;
  }

  #value(pointer: string): void {
    this.#whitespace();
    const character = this.#text[this.#index];
    if (character === "{") this.#object(pointer);
    else if (character === "[") this.#array(pointer);
    else if (character === "\"") this.#string();
    else this.#literal();
  }

  #object(pointer: string): void {
    this.#enterContainer();
    try {
      this.#expect("{");
      const seen = new Set<string>();
      this.#whitespace();
      if (this.#text[this.#index] === "}") {
        this.#index += 1;
        return;
      }
      for (;;) {
        this.#whitespace();
        const key = this.#string();
        if (seen.has(key)) this.duplicates.push({ pointer, key });
        seen.add(key);
        this.#whitespace();
        this.#expect(":");
        this.#value(`${pointer}/${escapePointerToken(key)}`);
        this.#whitespace();
        if (this.#text[this.#index] === ",") {
          this.#index += 1;
          continue;
        }
        this.#expect("}");
        return;
      }
    } finally {
      this.#leaveContainer();
    }
  }

  #array(pointer: string): void {
    this.#enterContainer();
    try {
      this.#expect("[");
      this.#whitespace();
      if (this.#text[this.#index] === "]") {
        this.#index += 1;
        return;
      }
      for (let element = 0; ; element += 1) {
        this.#value(`${pointer}/${element}`);
        this.#whitespace();
        if (this.#text[this.#index] === ",") {
          this.#index += 1;
          continue;
        }
        this.#expect("]");
        return;
      }
    } finally {
      this.#leaveContainer();
    }
  }

  /** The decoded string; `JSON.parse` of the literal handles every escape, so `"a"` and `"a"` are the same key. */
  #string(): string {
    const start = this.#index;
    this.#expect("\"");
    while (this.#index < this.#text.length && this.#text[this.#index] !== "\"") {
      this.#index += this.#text[this.#index] === "\\" ? 2 : 1;
    }
    this.#expect("\"");
    return JSON.parse(this.#text.slice(start, this.#index)) as string;
  }

  /** Numbers, `true`, `false`, and `null` hold no keys; they only need to be skipped. */
  #literal(): void {
    const start = this.#index;
    while (this.#index < this.#text.length && !",]} \t\n\r".includes(this.#text[this.#index]!)) this.#index += 1;
    if (this.#index === start) throw new SyntaxError(`Unexpected end of JSON at position ${start}`);
  }
}

/** Every repeated object key in `text`, in document order. `text` must be JSON; invalid input throws a SyntaxError. */
export function findDuplicateJsonKeys(text: string): DuplicateJsonKey[] {
  const scanner = new Scanner(text);
  scanner.scan();
  return scanner.duplicates;
}

/** `JSON.parse`, except that a repeated object key anywhere is an error naming its path. */
export function parseJsonWithoutDuplicateKeys(text: string, label: string): unknown {
  const value: unknown = JSON.parse(text);
  const duplicates = findDuplicateJsonKeys(text);
  if (duplicates.length > 0) throw new Error(`${label}: ${duplicates.map(describeDuplicateJsonKey).join("; ")}`);
  return value;
}

/**
 * The error for a request body that repeats a key, or undefined when it repeats none. Text that is not JSON is
 * left to the caller's own parser, which reports it in its usual shape.
 */
export function duplicateJsonKeyError(text: string, label: string): string | undefined {
  try {
    JSON.parse(text);
  } catch {
    return undefined;
  }
  try {
    const duplicates = findDuplicateJsonKeys(text);
    return duplicates.length === 0 ? undefined : `${label}: ${duplicates.map(describeDuplicateJsonKey).join("; ")}`;
  } catch (error) {
    return `${label}: ${error instanceof Error ? error.message : "could not validate duplicate keys"}`;
  }
}
