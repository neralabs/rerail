
export function test(): string {
  class Nested {
    public method() {
      return 'value';
    }
  }
  return new Nested().method();
}
      