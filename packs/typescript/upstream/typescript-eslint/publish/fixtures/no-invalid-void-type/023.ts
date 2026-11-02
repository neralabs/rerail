
class SomeClass {
  'f'(): void;
  'f'(x: string): string;
  'f'(x?: string): string | void {
    if (x !== undefined) {
      return x;
    }
  }
}
    