
class SomeClass {
  1(): void;
  1(x: string): string;
  1(x?: string): string | void {
    if (x !== undefined) {
      return x;
    }
  }
}
    