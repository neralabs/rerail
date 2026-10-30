
export type ArrayUgfxb<Cpcj> = Cpcj extends (arf0: Array<infer D>) => any
  ? D[]
  : Cpcj extends (...arff: infer D) => any
    ? D
    : never;
    