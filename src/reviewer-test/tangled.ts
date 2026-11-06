// Reviewer test (W4): a deliberately branchy function, so the complexity rule has something to find. Do not merge.
export function tangled(a: number, b: number, c: boolean): number {
  if (a > 1 && b > 2) return 1;
  if (a < 0 || b < 0) return 2;
  for (let i = 0; i < a; i++) {
    if (c) return 3;
  }
  while (b-- > 0) {
    if (a === b) return 4;
  }
  return c ? 5 : a > b ? 6 : 7;
}
