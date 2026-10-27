/**
 * Bladestan's coordinates: where a span is in Bladestan's units, from the engine's UTF-8 byte offsets (what the gates
 * compare an engine finding by) and from a Bladestan error (what the recorder records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";

/**
 * Bladestan reports an error's template line (its `template_line` metadata) only: PHPStan's own line is the caller's,
 * and the compiled PHP's positions are mapped back to template lines alone. No column and no end.
 */
export const BLADESTAN_SPAN_PARTS: SpanParts = { endLine: false, endCol: false };

/**
 * The 0-based line of a UTF-8 byte offset, as Bladestan numbers a template's lines: FileNameAndLineNumberAddingPreCompiler
 * splits the template at PHP_EOL, `\n` on the platforms the gates run on, so `\r\n` ends a line at its `\n` and a lone
 * `\r` ends none.
 */
export function bladestanLine(source: string, byteOffset: number): number {
  const prefix = Buffer.from(source, "utf8").subarray(0, byteOffset).toString("utf8");
  return prefix.match(/\n/gu)?.length ?? 0;
}

/** An engine byte span of `source` in Bladestan's coordinates: its start line, and nothing else, as Bladestan reports no more. */
export function bladestanSpan(source: string, startByte: number, _endByte: number, _parts: SpanParts = BLADESTAN_SPAN_PARTS): Span {
  return { startLine: bladestanLine(source, startByte), startCol: null, endLine: null, endCol: null };
}

/** A Bladestan error's template line (1-based) as a span, made 0-based. */
export function templateLineSpan(error: { templateLine: number }): Span {
  return { startLine: error.templateLine - 1, startCol: null, endLine: null, endCol: null };
}
