/**
 * Psalm's coordinates: where a span is in Psalm's units, from the engine's UTF-8 byte offsets (what the gates compare an
 * engine finding by) and from a Psalm issue's own byte offsets (what the recorder records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";
import { byteLineColumn } from "./bandit.js";

/** Psalm reports every issue's selection whole: its start and its end, as byte offsets and as lines and columns. */
export const PSALM_SPAN_PARTS: SpanParts = { endLine: true, endCol: true };

/**
 * A byte span of `source` in Psalm's coordinates: 0-based lines broken where PHP's tokenizer breaks them (`\r\n`, `\r`,
 * `\n`; Psalm's line_from is nikic/php-parser's line, which counts them), and UTF-8 byte columns, as Psalm counts
 * column_from and column_to (CodeLocation: byte offsets less the line's start), with the end parts `parts` keeps (both).
 * The recorder reads a Psalm issue (`--output-format=json`) through it from the issue's `from` and `to`, the byte
 * offsets of its selection: Psalm's own column_from counts from the last `\n` before the selection, so on a file's
 * first line, which has none, it is one short of every other line's, and the offsets have no such exception.
 */
export function psalmSpan(source: string, startByte: number, endByte: number, parts: SpanParts = PSALM_SPAN_PARTS): Span {
  const start = byteLineColumn(source, startByte);
  const end = byteLineColumn(source, endByte);
  return { startLine: start.line, startCol: start.column, endLine: parts.endLine ? end.line : null, endCol: parts.endCol ? end.column : null };
}
