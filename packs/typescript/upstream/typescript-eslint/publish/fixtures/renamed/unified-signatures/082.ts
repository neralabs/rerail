
interface F {
  e(p: string | number): void;
  e(p: number | boolean): void;
  e(p: symbol): string;
}
      