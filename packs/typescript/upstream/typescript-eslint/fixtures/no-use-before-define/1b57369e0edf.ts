
const obj = {
  foo: 'foo-value',
  bar: 'bar-value',
} satisfies {
  [key in 'foo' | 'bar']: `${key}-value`;
};
      