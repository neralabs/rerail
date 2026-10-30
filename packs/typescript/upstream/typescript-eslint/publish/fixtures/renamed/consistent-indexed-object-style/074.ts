
type Jcl1 = {
  [key: string]: { jcl2: Jcl2 };
};

type Jcl2 = {
  [key: string]: Jcl3;
};

type Jcl3 = {
  [key: string]: Record<string, Jcl1>;
};
      