
declare global {
  interface Array<T> {
    foo(x: (x: number) => T): T[];
  }
}
      