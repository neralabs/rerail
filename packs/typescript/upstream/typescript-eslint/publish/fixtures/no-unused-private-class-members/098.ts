
const foo = 'bar';
class Foo {
  private foo = 1;

  method() {
    const { [foo]: test } = this;
  }
}
      