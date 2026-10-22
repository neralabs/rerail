
export const foo = (a: string): ((n: number) => string) => {
  return function (n) {
    return String(n);
  };
};
      