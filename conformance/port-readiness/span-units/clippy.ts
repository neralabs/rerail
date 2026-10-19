/**
 * Clippy's coordinates: where a span is in rustc's units, from the engine's UTF-8 byte offsets (what the gates compare
 * an engine finding by) and from a rustc JSON diagnostic span (what the recorder records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";

/**
 * The 0-based line and character column of a UTF-8 byte offset, as rustc counts them after it normalizes a source
 * file (rustc_span's `normalize_src`): a byte-order mark at the start is dropped, `\r\n` is one line break and a lone
 * `\r` none, and every Unicode scalar value is one column.
 */
export function rustcPosition(source: string, byteOffset: number): { line: number; column: number } {
  const bytes = Buffer.from(source, "utf8");
  const prefix = bytes.subarray(0, Math.min(byteOffset, bytes.length)).toString("utf8");
  const lineStart = prefix.lastIndexOf("\n") + 1;
  const line = lineStart === 0 ? 0 : prefix.slice(0, lineStart).split("\n").length - 1;
  const text = prefix.slice(lineStart);
  return { line, column: [...(line === 0 ? text.replace(/^\uFEFF/u, "") : text)].length };
}

/** An engine byte span of `source` in rustc's coordinates, with the end parts `parts` keeps. */
export function clippySpan(source: string, startByte: number, endByte: number, parts: SpanParts): Span {
  const start = rustcPosition(source, startByte);
  const end = rustcPosition(source, endByte);
  return { startLine: start.line, startCol: start.column, endLine: parts.endLine ? end.line : null, endCol: parts.endCol ? end.column : null };
}

/** A rustc JSON diagnostic span: 1-based lines and character columns, end exclusive. */
export interface RustcSpan {
  line_start: number;
  column_start: number;
  line_end: number;
  column_end: number;
}

/** A rustc span as a span, made 0-based, with the end parts `parts` keeps. */
export function rustcSpan(span: RustcSpan, parts: SpanParts): Span {
  return {
    startLine: span.line_start - 1,
    startCol: span.column_start - 1,
    endLine: parts.endLine ? span.line_end - 1 : null,
    endCol: parts.endCol ? span.column_end - 1 : null,
  };
}
