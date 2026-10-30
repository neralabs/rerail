
type Bar<K extends string = never> = {
  [k in K]: Bar;
};
    