
export type T = {
  new (): T;
  new (arg: number): T;
  new <T>(arg: number): T;
};
    