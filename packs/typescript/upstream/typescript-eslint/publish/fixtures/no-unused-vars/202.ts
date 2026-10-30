
const foo: number = 1;

export type Foo = (typeof foo | string) & { __brand: 'foo' };
      