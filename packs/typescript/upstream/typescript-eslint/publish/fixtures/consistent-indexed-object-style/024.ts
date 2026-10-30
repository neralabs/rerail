
type Foo1 = {
  [key: string]: Foo2;
};

type Foo2 = {
  [key: string]: Foo3;
};

type Foo3 = {
  [key: string]: Foo1;
};
    