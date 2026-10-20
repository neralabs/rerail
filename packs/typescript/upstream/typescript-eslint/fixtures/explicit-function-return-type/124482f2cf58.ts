
interface Foo {
  foo: string;
  arrowFn: () => string;
}

function foo(): Foo {
  return {
    foo: 'foo',
    arrowFn: () => 'test',
  };
}
      