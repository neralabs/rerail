/**
 * PHP_CodeSniffer's coordinates: where a span is in PHP_CodeSniffer's units, from the engine's UTF-8 byte offsets (what
 * the gates compare an engine finding by) and from a PHP_CodeSniffer message (what the recorder records upstream's answer
 * as).
 */
import type { Span, SpanParts } from "../types.js";

/** PHP_CodeSniffer reports a message's line and column only: every message is a point, with no end. */
export const PHPCS_SPAN_PARTS: SpanParts = { endLine: false, endCol: false };

/**
 * The 0-based line and code-point column of a UTF-8 byte offset, as PHP_CodeSniffer counts them with a tab width of 0
 * (its default, and its own unit tests'): lines break at `\r\n`, `\r` and `\n`, and every character is one column, a tab
 * and a byte-order mark included, as its tokenizer measures token lengths with iconv_strlen.
 */
export function phpcsPosition(source: string, byteOffset: number): { line: number; column: number } {
  const prefix = Buffer.from(source, "utf8").subarray(0, byteOffset).toString("utf8");
  let line = 0;
  let lineStart = 0;
  for (let index = 0; index < prefix.length; index += 1) {
    const char = prefix[index];
    if (char === "\n" || (char === "\r" && prefix[index + 1] !== "\n")) {
      line += 1;
      lineStart = index + 1;
    }
  }
  return { line, column: [...prefix.slice(lineStart)].length };
}

/** An engine byte span of `source` in PHP_CodeSniffer's coordinates: its start, and the end parts `parts` keeps (none). */
export function phpcsSpan(source: string, startByte: number, endByte: number, parts: SpanParts = PHPCS_SPAN_PARTS): Span {
  const start = phpcsPosition(source, startByte);
  const end = phpcsPosition(source, endByte);
  return { startLine: start.line, startCol: start.column, endLine: parts.endLine ? end.line : null, endCol: parts.endCol ? end.column : null };
}

/** A PHP_CodeSniffer message's position (`--report=json`: 1-based `line` and `column`) as a span, made 0-based. */
export function messageSpan(message: { line: number; column: number }): Span {
  return { startLine: message.line - 1, startCol: message.column - 1, endLine: null, endCol: null };
}
