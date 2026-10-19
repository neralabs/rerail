
interface Foo {
  [a: string]: number;

  a: x;
  b: x;
  c: x;

  c(): void;
  b(): void;
  a(): void;

  (): Baz;

  new (): Bar;
}
      