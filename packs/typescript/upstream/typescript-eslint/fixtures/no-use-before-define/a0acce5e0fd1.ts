
const foo = {
  bar: 'bar',
} satisfies {
  bar: typeof baz;
};

const baz = '';
      