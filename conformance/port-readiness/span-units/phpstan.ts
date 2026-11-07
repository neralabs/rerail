/**
 * PHPStan's coordinates: where a span is in PHPStan's units, from the engine's UTF-8 byte offsets (what the gates compare
 * an engine finding by) and from a PHPStan error (what the recorder records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";

/** PHPStan reports an error's line alone (`--error-format=json`: `line`): no column and no end, so only the start line is compared. */
export const PHPSTAN_SPAN_PARTS: SpanParts = { endLine: false, endCol: false };

/**
 * The 0-based line of a UTF-8 byte offset, as PHP's tokenizer numbers lines (PHPStan reads the line of a node off
 * nikic/php-parser, which takes it from the tokenizer): lines break at `\r\n`, `\r` and `\n`.
 */
export function phpstanLine(source: string, byteOffset: number): number {
  const prefix = Buffer.from(source, "utf8").subarray(0, byteOffset).toString("utf8");
  let line = 0;
  // The `\r` lookahead reads `source`: an offset between `\r` and `\n` cuts the prefix there, and `\r\n` is one break.
  for (let index = 0; index < prefix.length; index += 1) {
    const char = prefix[index];
    if (char === "\n" || (char === "\r" && source[index + 1] !== "\n")) line += 1;
  }
  return line;
}

/** An engine byte span of `source` in PHPStan's coordinates: its start line, with no column and no end, whatever `parts` asks. */
export function phpstanSpan(source: string, startByte: number, _endByte: number, _parts: SpanParts = PHPSTAN_SPAN_PARTS): Span {
  return { startLine: phpstanLine(source, startByte), startCol: null, endLine: null, endCol: null };
}

/** A PHPStan error's position (its 1-based `line`) as a span, made 0-based. */
export function errorSpan(error: { line: number }): Span {
  return { startLine: error.line - 1, startCol: null, endLine: null, endCol: null };
}
