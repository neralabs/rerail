/**
 * Python's `fnmatch.fnmatchcase` (CPython 3.12 and 3.13, `Lib/fnmatch.py` `translate`), as SQLFluff matches rule
 * references and Bandit, on POSIX where `fnmatch` does not fold case, matches excluded paths: `*` (any run of
 * characters, `/` and line breaks included), `?`, `[...]` and `[!...]`; everything else literal.
 */

/** Characters `chars` as a regular expression matching them literally, each escaped by its code point. */
const literal = (chars: readonly string[]): string => chars.map(char => `\\u{${char.codePointAt(0)!.toString(16)}}`).join("");

/**
 * The literal runs of an `fnmatch` class body (after any `!`), a range between each two, as `fnmatch.translate` splits
 * them: a `-` is a range only after the body's first character and past the character after the range before it, and
 * a range running backwards is dropped with its ends.
 */
function classChunks(body: readonly string[]): string[][] {
  const chunks: string[][] = [];
  let from = 0;
  for (let hyphen = body.indexOf("-", 1); hyphen !== -1; hyphen = body.indexOf("-", hyphen + 3)) {
    chunks.push(body.slice(from, hyphen));
    from = hyphen + 1;
  }
  const last = body.slice(from);
  if (last.length > 0) chunks.push(last);
  else chunks.at(-1)!.push("-");
  for (let index = chunks.length - 1; index > 0; index -= 1) {
    const before = chunks[index - 1]!;
    const after = chunks[index]!;
    if (before.at(-1)! > after[0]!) chunks.splice(index - 1, 2, [...before.slice(0, -1), ...after.slice(1)]);
  }
  return chunks;
}

/** `fnmatch`'s `[...]` class starting at `start`: its end and its regular expression, or undefined when it is not closed. */
function globClass(chars: readonly string[], start: number): { end: number; source: string } | undefined {
  let end = start + 1;
  if (chars[end] === "!") end += 1;
  if (chars[end] === "]") end += 1;
  end = chars.indexOf("]", end);
  if (end === -1) return undefined;
  const negated = chars[start + 1] === "!";
  const ranges = classChunks(chars.slice(negated ? start + 2 : start + 1, end)).map(literal).join("-");
  // A class left empty is `[]`, matching nothing, or `[^]`, matching anything: fnmatch's `(?!)` and `.`.
  return { end, source: `[${negated ? "^" : ""}${ranges}]` };
}

/** `glob` as `fnmatch.fnmatchcase` matches it, as a regular expression matching whole strings. */
export function fnmatchPattern(glob: string): RegExp {
  const chars = Array.from(glob);
  let source = "";
  for (let index = 0; index < chars.length; index += 1) {
    const char = chars[index]!;
    const found = char === "[" ? globClass(chars, index) : undefined;
    if (found !== undefined) {
      source += found.source;
      index = found.end;
    } else {
      source += char === "*" ? ".*" : char === "?" ? "." : literal([char]);
    }
  }
  return new RegExp(`^${source}$`, "su");
}
