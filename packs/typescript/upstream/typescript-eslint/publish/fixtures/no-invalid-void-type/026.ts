
declare module foo {
  function f(): void;
  function f(x: string): string;
  function f(x?: string): string | void {
    if (x !== undefined) {
      return x;
    }
  }
}
    