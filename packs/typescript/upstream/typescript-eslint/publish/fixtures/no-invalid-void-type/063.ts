
class SomeClass {
  f(x?: string): string | void {
    if (x !== undefined) {
      return x;
    }
  }
}
      