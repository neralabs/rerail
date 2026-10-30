
type LfzzwutaovaVmat<R> = {
  [M in keyof R]: R[M] extends string ? void : string;
};
      