/**
 * Bandit's coordinates: where a span is in Bandit's units, from the engine's UTF-8 byte offsets (what the gates
 * compare an engine finding by) and from B613's own numbering (what the recorder records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";

/**
 * The end parts Bandit reports for a test: a start line, a start column and an end column, but no end line (its
 * `line_range` is the node's context range, which for a string is its parent's); B613 (trojansource) reports a start
 * only.
 */
export function banditSpanParts(testId: string): SpanParts {
  return testId.toUpperCase() === "B613" ? { endLine: false, endCol: false } : { endLine: false, endCol: true };
}

/**
 * The 0-based line and UTF-8 byte column of a byte offset in `source`, with lines broken where CPython's tokenizer
 * breaks them (`\r\n`, `\r`, `\n`): the coordinates of Python's `ast` positions.
 */
export function byteLineColumn(source: string, byteOffset: number): { line: number; column: number } {
  const bytes = Buffer.from(source, "utf8");
  let line = 0;
  let lineStart = 0;
  for (let index = 0; index < Math.min(byteOffset, bytes.length); index += 1) {
    const byte = bytes[index]!;
    if (byte === 0x0a || (byte === 0x0d && bytes[index + 1] !== 0x0a)) {
      line += 1;
      lineStart = index + 1;
    }
  }
  return { line, column: byteOffset - lineStart };
}

/** An engine byte span of `source` in Bandit's coordinates, with the end parts `parts` keeps (`banditSpanParts`). */
export function banditSpan(source: string, startByte: number, endByte: number, parts: SpanParts): Span {
  const start = byteLineColumn(source, startByte);
  const end = byteLineColumn(source, endByte);
  return { startLine: start.line, startCol: start.column, endLine: parts.endLine ? end.line : null, endCol: parts.endCol ? end.column : null };
}

/**
 * The Bandit release whose B613 (trojansource) line numbering trojansourcePoint follows, as
 * bandit/plugins/trojansource.py reads the file: it counts `str.splitlines()` lines, which break on `\r\n`, `\n`, `\r`,
 * \v, \f, \x1c-\x1e, \x85, U+2028 and U+2029. Releases up to 1.9.3 number lines differently, so a pack that ports B613
 * on another release does not run until trojansource.py is read at that pin.
 */
export const TROJANSOURCE_RELEASE = "1.9.4";
const SPLITLINES_BREAKS = "\\r\\n|[\\n\\r\\v\\f\\x1c\\x1d\\x1e\\x85\\u2028\\u2029]";

/**
 * B613 reports a `str.splitlines()` line and a 1-based code-point column, with no end. The physical line and the UTF-8
 * byte column of that point.
 */
export function trojansourcePoint(source: string, line: number, column: number): { line: number; column: number } {
  const breaks = new RegExp(SPLITLINES_BREAKS, "gu");
  let start = 0;
  for (let current = 1; current < line; current += 1) {
    const match = breaks.exec(source);
    if (match === null) break;
    start = match.index + match[0].length;
  }
  const offset = start + [...source.slice(start)].slice(0, column - 1).join("").length;
  return byteLineColumn(source, Buffer.byteLength(source.slice(0, offset), "utf8"));
}
