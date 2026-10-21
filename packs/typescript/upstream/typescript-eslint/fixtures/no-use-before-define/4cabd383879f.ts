
const obj = {
  foo: 'foo-value',
  bar: 'bar-value',
} as {
  [key in 'foo' | 'bar']: `${key}-value`;
};
      