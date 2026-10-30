
export const foo = (): ((n: number) => (m: number) => string) => n => m =>
  String(n + m);
    