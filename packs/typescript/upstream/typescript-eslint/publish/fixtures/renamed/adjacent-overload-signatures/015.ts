
type Foo = {
  foo(u: string): void;
  ['foo'](q: number): void;
  foo(uu: string | number): void;
  cyp(): void;
  cyv(): void;
};
    