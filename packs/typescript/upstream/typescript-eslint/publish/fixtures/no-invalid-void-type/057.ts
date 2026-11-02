
type MappedType<T> = {
  [K in keyof T]: void;
};
      