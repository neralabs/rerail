
type Foos<K extends string = never> = {
  [k in K]: { foo: Foo };
};

type Foo = Foos;
      