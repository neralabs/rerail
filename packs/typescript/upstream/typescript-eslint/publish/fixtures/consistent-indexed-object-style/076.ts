
type Foos<K extends string = never> = {
  [k in K]: Foo[];
};

type Foo = Foos;
      