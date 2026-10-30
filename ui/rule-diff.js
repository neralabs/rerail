/**
 * A rule's content as a person reads it before approving it, and what changed against the content approved before.
 * Pure: no DOM. The text goes on the page only through `textContent`.
 */

/**
 * Invisible characters (format controls such as zero-width spaces and bidi overrides, private use, unassigned code
 * points, tag characters) written as `\uXXXX` or `\u{XXXXX}`, so what a person reads is everything the rule holds.
 * The CLI's `rerail rules approve` prints rules the same way.
 */
export function escapeInvisible(text) {
  return String(text).replace(/[\p{Cf}\p{Co}\p{Cn}]/gu, character => {
    const code = character.codePointAt(0);
    return code > 0xffff ? `\\u{${code.toString(16)}}` : `\\u${code.toString(16).padStart(4, "0")}`;
  });
}

/** `value` with every object's keys sorted, so two versions of a rule line up whatever order their keys came in. */
function sortedKeys(value) {
  if (Array.isArray(value)) return value.map(sortedKeys);
  if (value === null || typeof value !== "object") return value;
  return Object.fromEntries(Object.keys(value).sort().map(key => [key, sortedKeys(value[key])]));
}

/** A rule as pretty-printed JSON, keys sorted, invisible characters escaped. */
export const visibleJson = value => escapeInvisible(JSON.stringify(sortedKeys(value), null, 2));

/**
 * Every line of `after`, in order, with the lines of `before` it no longer has in their place: `{ op, text }`, `op`
 * "same", "added" (only in `after`) or "removed" (only in `before`). A longest common subsequence of the lines; where
 * a line was replaced, the removed line comes first.
 */
export function lineDiff(before, after) {
  const a = before.split("\n"), b = after.split("\n");
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start += 1;
  let endA = a.length, endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) { endA -= 1; endB -= 1; }
  const n = endA - start, m = endB - start;
  // lcs[i][j]: the longest common subsequence of a[start + i..endA) and b[start + j..endB).
  const lcs = Array.from({ length: n + 1 }, () => new Array(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i -= 1) {
    for (let j = m - 1; j >= 0; j -= 1) {
      lcs[i][j] = a[start + i] === b[start + j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }
  const lines = a.slice(0, start).map(text => ({ op: "same", text }));
  let i = 0, j = 0;
  while (i < n || j < m) {
    if (i < n && j < m && a[start + i] === b[start + j]) { lines.push({ op: "same", text: a[start + i] }); i += 1; j += 1; }
    else if (i < n && (j === m || lcs[i + 1][j] >= lcs[i][j + 1])) { lines.push({ op: "removed", text: a[start + i] }); i += 1; }
    else { lines.push({ op: "added", text: b[start + j] }); j += 1; }
  }
  for (const text of a.slice(endA)) lines.push({ op: "same", text });
  return lines;
}
