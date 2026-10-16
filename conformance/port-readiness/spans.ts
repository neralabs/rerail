import type { Span } from "./types.js";

/** Every position upstream reports, as one comparable key; a part upstream does not report is null on both sides. */
export function spanKey(span: Span): string {
  return `${span.startLine}:${span.startCol}-${span.endLine}:${span.endCol}`;
}

/** The start position alone. */
export function startKey(span: Span): string {
  return `${span.startLine}:${span.startCol}`;
}

/** The spans of `left` that `right` does not account for, compared as multisets by `key`. */
export function unmatched(left: Span[], right: Span[], key: (span: Span) => string = spanKey): Span[] {
  const pool = new Map<string, number>();
  for (const span of right) pool.set(key(span), (pool.get(key(span)) ?? 0) + 1);
  return left.filter((span) => {
    const count = pool.get(key(span)) ?? 0;
    if (count === 0) return true;
    pool.set(key(span), count - 1);
    return false;
  });
}

/** A span for people: 1-based line:column, with only the parts upstream reports. */
export function formatSpan(span: Span): string {
  const point = (line: number | null, col: number | null) => (line === null ? "" : col === null ? `${line + 1}` : `${line + 1}:${col + 1}`);
  const end = point(span.endLine, span.endCol);
  return end === "" ? point(span.startLine, span.startCol) : `${point(span.startLine, span.startCol)}-${end}`;
}
