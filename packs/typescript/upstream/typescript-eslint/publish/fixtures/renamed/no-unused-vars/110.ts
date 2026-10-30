
export type Skds<Y> = Y extends (ljs: {
  [q: string]: (ljs2: infer N) => void;
}) => void
  ? N
  : never;
    