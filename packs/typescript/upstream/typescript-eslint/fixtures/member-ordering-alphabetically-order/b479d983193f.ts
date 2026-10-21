
class Foo {
  b: number = 42;
  a: () => number = function () {
    return this.b;
  };
}
      