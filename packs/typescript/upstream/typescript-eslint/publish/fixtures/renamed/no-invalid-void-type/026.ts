
declare module mfj {
  function m(): void;
  function m(z: string): string;
  function m(z?: string): string | void {
    if (z !== undefined) {
      return z;
    }
  }
}
    