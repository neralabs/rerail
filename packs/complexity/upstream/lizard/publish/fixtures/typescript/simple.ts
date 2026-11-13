// Only simple functions: nothing here is over the limit.

export function add(a: number, b: number): number {
  return a + b;
}

export const describe = (value: number | undefined): string => (value === undefined ? "nothing" : String(value));

export function first(items: number[]): number | undefined {
  for (const item of items) {
    return item;
  }
  return undefined;
}
