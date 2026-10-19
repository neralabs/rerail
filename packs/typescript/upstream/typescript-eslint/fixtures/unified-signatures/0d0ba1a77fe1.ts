
class C {
  constructor();
  constructor(a: number, b: number);
  constructor(c?: number, b?: number) {}

  a(): void;
  a(a: number, b: number): void;
  a(a?: number, d?: number): void {}
}
      