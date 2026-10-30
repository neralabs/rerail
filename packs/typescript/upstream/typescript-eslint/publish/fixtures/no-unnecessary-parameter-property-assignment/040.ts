
class Foo {
  constructor(private foo: string) {
    this.foo = foo;
  }
  init = class Bar {
    constructor(private foo: string) {
      this.foo = foo;
    }
  };
}
      