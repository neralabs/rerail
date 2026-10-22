
interface Bar {
  type: typeof Foo.Bar.Baz;
}

const Foo = {
  Bar: {
    Baz: 1,
  },
};
      