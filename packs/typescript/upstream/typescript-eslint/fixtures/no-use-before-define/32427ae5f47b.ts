
const obj = {
  foo: {
    foo: 'foo',
  } as {
    [key in 'foo' | 'bar']: key;
  },
};
      