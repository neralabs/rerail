
interface Foo {
  [foo](bar: string): one;
  [foo](bar: number, baz: string): two;
  [foo](): three;
  bar(arg: string): void;
  bar(baz: number): Foo;
}
      