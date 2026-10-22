
type Foo1 = {
  [key: string]: { foo2: Foo2 };
};

type Foo2 = {
  [key: string]: Foo3;
};

type Foo3 = {
  [key: string]: Record<string, Foo1>;
};
      