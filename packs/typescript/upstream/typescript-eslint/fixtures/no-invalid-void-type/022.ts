
class SomeClass {
  [Symbol.iterator](): void;
  [Symbol.iterator](x: string): string;
  [Symbol.iterator](x?: string): string | void {
    if (x !== undefined) {
      return x;
    }
  }
}
    