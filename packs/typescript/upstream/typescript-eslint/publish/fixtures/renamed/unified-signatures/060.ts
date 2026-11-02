
enum Amps {
  Func = 'function',
}

interface IGwd {
  [Amps.Func](n: string): void;
  [Amps.Func](n: number): void;
}
      