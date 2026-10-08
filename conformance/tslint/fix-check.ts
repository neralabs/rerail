/** Suggested fixes emitted by a port, applied the way TSLint applies them and compared with its `.fix` files. */

/** A violation's suggested fix: replace `[startByte, endByte)` of the fixture source with `text`. */
export interface EmittedFix {
  startByte: number;
  endByte: number;
  text: string;
}

/** Emitted fixes applied to a fixture, compared with TSLint's `<file>.fix` (its expected fixed text). */
export interface FixCheck {
  emitted: number;
  matched: boolean;
  /** First differing line (0-based) of the fixed text, with both versions of that line. */
  firstDiff?: { line: number; expected: string; actual: string };
}

/** TSLint's `Replacement.applyAll`: sort by end, then start, descending and apply each to the running text. */
export function applyFixes(source: string, fixes: EmittedFix[]): string {
  const sorted = [...fixes].sort((a, b) => b.endByte - a.endByte || b.startByte - a.startByte);
  let bytes = Buffer.from(source, "utf8");
  for (const fix of sorted) bytes = Buffer.concat([bytes.subarray(0, fix.startByte), Buffer.from(fix.text, "utf8"), bytes.subarray(fix.endByte)]);
  return bytes.toString("utf8");
}

export function checkFix(source: string, fixes: EmittedFix[], expected: string): FixCheck {
  const actual = applyFixes(source, fixes);
  if (actual === expected) return { emitted: fixes.length, matched: true };
  const actualLines = actual.split("\n");
  const expectedLines = expected.split("\n");
  let line = 0;
  while (actualLines[line] === expectedLines[line]) line += 1;
  return { emitted: fixes.length, matched: false, firstDiff: { line, expected: expectedLines[line] ?? "(end of file)", actual: actualLines[line] ?? "(end of file)" } };
}
