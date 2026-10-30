
interface Foo {
  x(
    args: Pick<
      Bar,
      'one' | 'two' | 'three'
    >,
  ): Baz;
  y(
    foo: string,
    bar: number,
  ): void;
}
      