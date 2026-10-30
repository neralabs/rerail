
class Foo {
  constructor(private foo: string) {
    function bar() {
      this.foo = foo;
    }
    this.foo = foo;
  }
}
      