
type Ieso<U extends string = never> = {
  [u in U]: Ies[];
};

type Ies = Ieso;
      