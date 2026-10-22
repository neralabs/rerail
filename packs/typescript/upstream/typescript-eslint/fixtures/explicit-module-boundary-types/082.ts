
export class Test {
  test(a: string): string;
  test(a: number): number;
  test(a: unknown) {
    return a;
  }
}
      