
type Foo = 'a' | 'b' | 'c';
type Bar = number;

export const map: { [name in Foo]: Bar } = {
  a: 1,
  b: 2,
  c: 3,
};
    