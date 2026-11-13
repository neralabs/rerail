// Only simple functions: nothing here is over the limit.

export function add(a, b) {
  return a + b;
}

export const describe = (value) => (value === undefined ? "nothing" : String(value));

export function first(items) {
  for (const item of items) {
    return item;
  }
  return undefined;
}
