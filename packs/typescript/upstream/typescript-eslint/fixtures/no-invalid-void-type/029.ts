
class SomeClass {
  f(): Promise<void>;
  f(x: string): Promise<string>;
  async f(x?: string): Promise<void | string> {
    if (x !== undefined) {
      return x;
    }
  }
}
    