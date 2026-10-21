
class A {
  private readonly foo: string = 'bar';
  constructor(foo: string) {
    function func() {
      this.foo = 'aa';
    }
  }
}
      