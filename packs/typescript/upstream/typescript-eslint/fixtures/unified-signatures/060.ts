
enum Enum {
  Func = 'function',
}

interface IFoo {
  [Enum.Func](x: string): void;
  [Enum.Func](x: number): void;
}
      