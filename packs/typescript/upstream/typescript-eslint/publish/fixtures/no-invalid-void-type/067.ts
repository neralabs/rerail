
class SomeClass {
  f(): void;
  f(x: string): string | void;
  f(x?: string): string | void {
    if (x !== undefined) {
      return x;
    }
  }
}
      