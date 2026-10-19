
interface I {
  f(x: string): void;
  /**
   * @deprecate
   */
  f(x: number): void;
  /**
   * @deprecate
   */
  f(x: boolean): void;
}
      