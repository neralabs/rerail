
class Foo {
  constructor(private foo: string) {
    this.foo = foo;
  }
  init = foo => {
    this.foo = foo;
  };
}
      