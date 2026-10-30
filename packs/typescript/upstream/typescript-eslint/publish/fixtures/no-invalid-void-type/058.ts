
type ConditionalType<T> = {
  [K in keyof T]: T[K] extends string ? void : string;
};
      