/**
 * Ruff's coordinates: where a span is in Ruff's units, from the engine's UTF-8 byte offsets (what the gates compare
 * an engine finding by) and from a Ruff diagnostic (what the recorder records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";

/** Ruff reports every finding's start and end. */
export const RUFF_SPAN_PARTS: SpanParts = { endLine: true, endCol: true };

/**
 * The 0-based line and code-point column of a UTF-8 byte offset, as Ruff counts them: lines break at `\r\n`, `\r` and
 * `\n`, every Unicode scalar value is one column, and a byte-order mark at the start of the file is not counted.
 */
export function codePointPosition(source: string, byteOffset: number): { line: number; column: number } {
  const bytes = Buffer.from(source, "utf8");
  const prefix = bytes.subarray(0, Math.min(byteOffset, bytes.length)).toString("utf8");
  let line = 0;
  let lineStart = 0;
  for (let index = 0; index < prefix.length; index += 1) {
    const char = prefix[index];
    if (char === "\n" || (char === "\r" && prefix[index + 1] !== "\n")) {
      line += 1;
      lineStart = index + 1;
    }
  }
  const text = prefix.slice(lineStart);
  const column = [...(line === 0 ? text.replace(/^\uFEFF/u, "") : text)].length;
  return { line, column };
}

/** An engine byte span of `source` in Ruff's coordinates, with the end parts `parts` keeps. */
export function ruffSpan(source: string, startByte: number, endByte: number, parts: SpanParts = RUFF_SPAN_PARTS): Span {
  const start = codePointPosition(source, startByte);
  const end = codePointPosition(source, endByte);
  return { startLine: start.line, startCol: start.column, endLine: parts.endLine ? end.line : null, endCol: parts.endCol ? end.column : null };
}

/** A Ruff diagnostic's range, as `ruff check --output-format json` writes it: 1-based rows and code-point columns. */
export interface RuffRange {
  location: { row: number; column: number } | null;
  end_location: { row: number; column: number } | null;
}

/** A Ruff diagnostic's range as a span: Ruff's 1-based rows and code-point columns, end exclusive, made 0-based. */
export function diagnosticSpan(diagnostic: RuffRange): Span {
  const start = diagnostic.location ?? { row: 1, column: 1 };
  const end = diagnostic.end_location;
  return { startLine: start.row - 1, startCol: start.column - 1, endLine: end === null ? null : end.row - 1, endCol: end === null ? null : end.column - 1 };
}
