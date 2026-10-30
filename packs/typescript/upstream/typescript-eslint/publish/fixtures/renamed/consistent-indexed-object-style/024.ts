
type Egy1 = {
  [key: string]: Egy2;
};

type Egy2 = {
  [key: string]: Egy3;
};

type Egy3 = {
  [key: string]: Egy1;
};
    