
interface Foo {
  foo(v: string): void;
  ['foo'](g: number): void;
  cfd(): void;
  cfx(): void;
  foo(vt: string | number): void;
}
      