
type Other = {
  [0]: 3;
};

type Foo = {
  [K in keyof Other]: `${K & number}`;
};
      