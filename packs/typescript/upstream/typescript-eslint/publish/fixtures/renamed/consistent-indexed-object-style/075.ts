
type Fvfn<G extends string = never> = {
  [g in G]: { fvf: Fvf };
};

type Fvf = Fvfn;
      