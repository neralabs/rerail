/**
 * stillat/blade-parser's coordinates: where a span is in the units its validation errors report, from the engine's
 * UTF-8 byte offsets (what the gates compare an engine finding by) and from a validation error (what the recorder
 * records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";

/** A validation error is compared by the start of the node it reports alone: the rules anchor a finding at a point. */
export const BLADE_PARSER_SPAN_PARTS: SpanParts = { endLine: false, endCol: false };

/**
 * The 0-based line and column of a UTF-8 byte offset, as stillat/blade-parser 2.1.0 numbers a node's start
 * (DocumentParser::prepareInput and lineColumnFromOffset). It normalizes `\r\n` and `\r` to `\n`, measures a node's
 * offset in characters (mb_* functions), and looks that offset up in a table of the `\n`s' byte offsets
 * (preg_match_all's): the line is one past the number of `\n`s at a byte offset below the node's character offset, and
 * the column the distance from the last of them, counted in the same mixed units. In ASCII text those are the plain
 * line and code-point column; after a multibyte character the node is read where its character count lands among the
 * byte offsets, as upstream reports it. The table's first entry is read as absent when it is at offset 0 (PHP's
 * `0 != null` is false), so the second line's columns then count from the template's start.
 */
export function bladeParserPosition(source: string, byteOffset: number): { line: number; column: number } {
  const text = Buffer.from(source.replace(/\r\n?/gu, "\n"), "utf8");
  const offset = [...Buffer.from(source, "utf8").subarray(0, byteOffset).toString("utf8").replace(/\r\n?/gu, "\n")].length;
  // The `\n`s' byte offsets in the normalized template, each with the column upstream's table gives it.
  const newlines: { index: number; char: number }[] = [];
  let previous: number | null = null;
  for (let index = text.indexOf(0x0a); index !== -1; index = text.indexOf(0x0a, index + 1)) {
    newlines.push({ index, char: previous === null || previous === 0 ? index + 1 : index - previous });
    previous = index;
  }
  if (newlines.length === 0) return { line: 0, column: offset };
  const nearest = newlines.findIndex((newline) => newline.index >= offset);
  if (nearest === -1) {
    const last = newlines[newlines.length - 1]!;
    return { line: newlines.length, column: offset - last.index - 1 };
  }
  const entry = newlines[nearest]!;
  return { line: nearest, column: entry.char - entry.index + offset - 1 };
}

/** An engine byte span of `source` in stillat/blade-parser's coordinates: its start, and the end parts `parts` keeps (none). */
export function bladeParserSpan(source: string, startByte: number, endByte: number, parts: SpanParts = BLADE_PARSER_SPAN_PARTS): Span {
  const start = bladeParserPosition(source, startByte);
  const end = bladeParserPosition(source, endByte);
  return { startLine: start.line, startCol: start.column, endLine: parts.endLine ? end.line : null, endCol: parts.endCol ? end.column : null };
}

/** A validation error's position (1-based start line, plus its shift line, and start column) as a span, made 0-based. */
export function validationSpan(error: { line: number; column: number }): Span {
  return { startLine: error.line - 1, startCol: error.column - 1, endLine: null, endCol: null };
}
