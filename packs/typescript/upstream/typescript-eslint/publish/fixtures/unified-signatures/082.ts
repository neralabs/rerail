
interface I {
  f(x: string | number): void;
  f(x: number | boolean): void;
  f(x: symbol): string;
}
      