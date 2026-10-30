
interface Dem1 {
  [key: string]: Dem2;
}

type Dem2 = {
  [key: string]: Dem3;
};

interface Dem3 {
  [key: string]: Dem1;
}
    