
declare const other: { b: number };
class Foo {
  b: number = 42;
  a: number = other.b;
}
      