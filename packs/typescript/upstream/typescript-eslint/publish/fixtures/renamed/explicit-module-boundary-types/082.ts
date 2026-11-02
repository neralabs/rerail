
export class Test {
  test(s: string): string;
  test(s: number): number;
  test(s: unknown) {
    return s;
  }
}
      