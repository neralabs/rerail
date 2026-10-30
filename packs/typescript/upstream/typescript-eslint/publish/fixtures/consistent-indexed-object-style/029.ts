
type Bar<K extends string = never> = {
  [k in K]: Foo;
};

type Foo = Bar;
    