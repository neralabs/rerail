
export function foo(): (n: number) => (m: number) => string {
  return function (n) {
    return function (m) {
      return String(n + m);
    };
  };
}
    