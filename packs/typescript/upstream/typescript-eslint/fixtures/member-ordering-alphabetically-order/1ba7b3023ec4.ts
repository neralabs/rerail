
declare const key: string;
class Foo {
  b: number = 42;
  a: number = this[key];
}
      