
class Foo {
  b: number = 42;
  a = class {
    value = this.b;
  };
}
      