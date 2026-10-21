
export function foo(outer: boolean) {
  if (outer) {
    return 'string';
  }
  return function (inner): void {};
}
      