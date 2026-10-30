
class Foo {
  constructor(private foo: string) {
    class Bar {
      constructor(private foo: string) {
        this.foo = foo;
      }
    }
    this.foo = foo;
  }
}
      