/**
 * ESLint's coordinates: where a span is in ESLint's units, from the engine's UTF-8 byte offsets (what the gates
 * compare an engine finding by) and from an ESLint message position (what the recorder records upstream's answer as).
 */
import type { Span, SpanParts } from "../types.js";

/** An ESLint message's position as the driver reports it: `[line, column, endLine, endColumn]`, 1-based, end exclusive. */
export type MessagePosition = [number, number, number | null, number | null];

/** The end parts ESLint reports for a rule: both ends, or neither when some finding on its own tests lacks them. */
export function eslintSpanParts(endsReported: boolean): SpanParts {
  return { endLine: endsReported, endCol: endsReported };
}

const LINE_BREAK = /\r\n|[\r\n\u2028\u2029]/gu;

/** Where each line starts in a source as ESLint reads it: without a leading BOM, broken at CRLF, CR, LF, LS and PS. */
export function eslintLineStarts(source: string): number[] {
  const text = source.startsWith("\uFEFF") ? source.slice(1) : source;
  return [0, ...[...text.matchAll(LINE_BREAK)].map((match) => match.index + match[0].length)];
}

/**
 * The 0-based line and UTF-16 column ESLint gives a UTF-8 byte offset of `source`: the leading BOM ESLint strips
 * shifts nothing after it, and lines break where ESLint's do.
 */
export function eslintPoint(source: string, byteOffset: number, starts: number[] = eslintLineStarts(source)): { line: number; column: number } {
  const offset = Math.max(0, Buffer.from(source, "utf8").subarray(0, byteOffset).toString("utf8").length - (source.startsWith("\uFEFF") ? 1 : 0));
  let line = 0;
  while (line + 1 < starts.length && starts[line + 1]! <= offset) line += 1;
  return { line, column: offset - starts[line]! };
}

/** The last source whose line starts were read: an engine run converts every finding of one file in a row. */
let lastLineStarts: { source: string; starts: number[] } | undefined;

/** An engine byte span of `source` in ESLint's coordinates, with the end parts `parts` keeps (`eslintSpanParts`). */
export function eslintSpan(source: string, startByte: number, endByte: number, parts: SpanParts): Span {
  if (lastLineStarts?.source !== source) lastLineStarts = { source, starts: eslintLineStarts(source) };
  const { starts } = lastLineStarts;
  const start = eslintPoint(source, startByte, starts);
  if (!parts.endLine && !parts.endCol) return { startLine: start.line, startCol: start.column, endLine: null, endCol: null };
  const end = eslintPoint(source, endByte, starts);
  return { startLine: start.line, startCol: start.column, endLine: parts.endLine ? end.line : null, endCol: parts.endCol ? end.column : null };
}

/** An ESLint finding position (1-based, exclusive end) as a 0-based span; without ends when upstream reports none for the rule. */
export function messageSpan([line, column, endLine, endColumn]: MessagePosition, ends: boolean): Span {
  return ends && endLine !== null && endColumn !== null
    ? { startLine: line - 1, startCol: column - 1, endLine: endLine - 1, endCol: endColumn - 1 }
    : { startLine: line - 1, startCol: column - 1, endLine: null, endCol: null };
}
