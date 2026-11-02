
export interface Foo {
  bar: string;
  baz: Foo['bar'];
}
    