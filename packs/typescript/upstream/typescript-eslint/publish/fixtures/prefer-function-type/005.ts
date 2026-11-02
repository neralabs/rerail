
interface Foo {
  bar: string;
}
interface Bar extends Function, Foo {
  (): void;
}
    