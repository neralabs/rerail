
class Foo {
  foo(o: string): void;
  ['foo'](l: number): void;
  foo(ot: string | number): void {}
  dvd(): void {}
  dvn(): void {}
}
    