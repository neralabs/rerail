
class Foo {
  constructor(
    public foo: number,
    bar: boolean,
  ) {
    if (bar) {
      this.foo += 1;
    } else {
      this.foo = foo;
    }
  }
}
    