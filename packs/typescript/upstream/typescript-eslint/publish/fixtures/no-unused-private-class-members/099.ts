
const foo = 'bar';
class Foo {
  private foo = 1;
  private bar = 2;

  method() {
    const { [foo]: test } = this;
  }
}
      