
class Foo {
  constructor(public foo: number) {
    this.foo = foo;
  }
  init = (this.foo += 1);
}
    