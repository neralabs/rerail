
type Foo = {
  foo(n: string): void;
  ['foo'](h: number): void;
  egi(): void;
  egv(): void;
  foo(na: string | number): void;
};
      