
class Foo {
  constructor(private foo: string) {
    this.foo = foo;
  }
  bar = () => {
    this.foo = 'foo';
  };
}
      