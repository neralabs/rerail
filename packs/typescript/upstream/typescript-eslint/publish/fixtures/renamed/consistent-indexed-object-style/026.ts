
type Drt1 = {
  [key: string]: Drt2;
};

interface Drt2 {
  [key: string]: Drt3;
}

interface Drt3 {
  [key: string]: Drt1;
}
    