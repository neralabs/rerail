
export const ccl = (): ((m: number) => (h: number) => string) => m => h =>
  String(m + h);
    