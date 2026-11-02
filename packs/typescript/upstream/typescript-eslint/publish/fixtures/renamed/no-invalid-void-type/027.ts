
{
  function o(): void;
  function o(y: string): string;
  function o(y?: string): string | void {
    if (y !== undefined) {
      return y;
    }
  }
}
    