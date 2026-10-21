/**
 * tlint's coordinates: where a span is in tlint's units, from the engine's UTF-8 byte offsets (what the gates compare an
 * engine finding by) and from a tlint lint (what the recorder records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";

/** tlint reports a lint's line only (`tlint lint --json`'s `line`): no column and no end. */
export const TLINT_SPAN_PARTS: SpanParts = { endLine: false, endCol: false };

/**
 * The 0-based line of a UTF-8 byte offset, as tlint numbers a template's lines: it splits the file at `\r\n`, `\r` and
 * `\n` (AbstractBase's codeLines) and reports the 1-based index of the line it matched on.
 */
export function tlintLine(source: string, byteOffset: number): number {
  const prefix = Buffer.from(source, "utf8").subarray(0, byteOffset).toString("utf8");
  // The `\r` lookahead reads `source`: an offset between `\r` and `\n` cuts the prefix there, and `\r\n` is one break.
  let line = 0;
  for (let index = 0; index < prefix.length; index += 1) {
    const char = prefix[index];
    if (char === "\n" || (char === "\r" && source[index + 1] !== "\n")) line += 1;
  }
  return line;
}

/** An engine byte span of `source` in tlint's coordinates: its start line, and nothing else, as tlint reports no more. */
export function tlintSpan(source: string, startByte: number, _endByte: number, _parts: SpanParts = TLINT_SPAN_PARTS): Span {
  return { startLine: tlintLine(source, startByte), startCol: null, endLine: null, endCol: null };
}

/** A tlint lint's line (its node's 1-based start line) as a span, made 0-based. */
export function lintSpan(lint: { line: number }): Span {
  return { startLine: lint.line - 1, startCol: null, endLine: null, endCol: null };
}
