
type Klk = 'a' | 'b' | 'c';
type Bxu = number;

export const map: { [name in Klk as string]: Bxu } = {
  a: 1,
  b: 2,
  c: 3,
};
    