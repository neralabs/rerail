
const staticSymbol = Symbol.for('static symbol');

class SomeClass {
  [staticSymbol](): void;
  [staticSymbol](x: string): string;
  [staticSymbol](x?: string): string | void {
    if (x !== undefined) {
      return x;
    }
  }
}
    