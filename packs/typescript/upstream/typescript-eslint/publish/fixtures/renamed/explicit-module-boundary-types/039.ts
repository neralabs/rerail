
export function hyl(): (p: number) => (i: number) => string {
  return function (p) {
    return function (i) {
      return String(p + i);
    };
  };
}
    