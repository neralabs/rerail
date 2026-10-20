
interface Foo {
  new (): Bar;

  a(): void;
  b(): void;
  c(): void;

  a: x;
  b: x;
  c: x;

  [a: string]: number;
  (): Baz;
}
      