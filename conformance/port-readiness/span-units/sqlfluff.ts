/**
 * SQLFluff's coordinates: where a span is in SQLFluff's units, from the engine's UTF-8 byte offsets (what the gates
 * compare an engine finding by) and from a SQLFluff violation (what the recorder records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";

/** SQLFluff reports a finding's start alone (`line_no`, `line_pos`), never its end. */
export const SQLFLUFF_SPAN_PARTS: SpanParts = { endLine: false, endCol: false };

/**
 * The 0-based line and character column of a UTF-8 byte offset, as SQLFluff counts them: its positions break lines at
 * `\r\n`, `\r` and `\n` (`SELECT 1;\rSELECT ifnull(a, 1)` reports CV02 at line 2), and `line_pos` counts the characters
 * (Unicode scalar values, Python `str` positions) before the offset on its line.
 */
function characterPosition(source: string, byteOffset: number): { line: number; column: number } {
  const prefix = Buffer.from(source, "utf8").subarray(0, Math.max(0, byteOffset)).toString("utf8");
  const lines = prefix.split(/\r\n|\r|\n/u);
  return { line: lines.length - 1, column: [...lines.at(-1)!].length };
}

/** An engine byte span of `source` in SQLFluff's coordinates, with the end parts `parts` keeps (none for SQLFluff). */
export function sqlfluffSpan(source: string, startByte: number, endByte: number, parts: SpanParts = SQLFLUFF_SPAN_PARTS): Span {
  const start = characterPosition(source, startByte);
  const end = parts.endLine || parts.endCol ? characterPosition(source, endByte) : null;
  return { startLine: start.line, startCol: start.column, endLine: parts.endLine ? end!.line : null, endCol: parts.endCol ? end!.column : null };
}

/** A SQLFluff violation's position as a span: its 1-based `line_no` and character `line_pos`, made 0-based. */
export function violationSpan(line: number, column: number): Span {
  return { startLine: line - 1, startCol: column - 1, endLine: null, endCol: null };
}
