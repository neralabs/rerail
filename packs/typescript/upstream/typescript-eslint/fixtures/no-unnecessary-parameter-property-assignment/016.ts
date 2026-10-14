
class Foo {
  constructor(public foo: number) {
    {
      const foo = 1;
      this.foo = foo;
    }
  }
}
    