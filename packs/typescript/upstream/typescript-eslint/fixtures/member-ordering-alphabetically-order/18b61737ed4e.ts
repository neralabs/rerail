
interface Foo {
  b(): void;
  a: b;
  [a: string]: number;
  new (): Bar;
  (): Baz;
}
      