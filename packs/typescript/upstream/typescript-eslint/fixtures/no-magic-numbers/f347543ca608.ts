
type Others = [['a'], ['b']];

type Foo = {
  [K in keyof Others[0]]: Others[K];
};
      