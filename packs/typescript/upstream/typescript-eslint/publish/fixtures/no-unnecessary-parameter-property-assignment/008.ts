
class Foo {
  constructor(private foo: string) {
    this.bar = () => {
      this.foo = foo;
    };
  }
}
    