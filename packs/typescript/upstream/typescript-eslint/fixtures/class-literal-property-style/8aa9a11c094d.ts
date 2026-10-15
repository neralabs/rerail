
class A {
  private readonly foo: string = 'bar';
  constructor(foo: string) {
    const bar = new (class {
      private readonly foo: string = 'baz';
      constructor() {
        this.foo = 'qux';
      }
    })();
    this['foo'] = foo;
  }
}
      