
class C {
  constructor();
  constructor(a: number, b: number);
  constructor(a?: number, b?: number) {}

  a(): void;
  a(a: number, b: number): void;
  a(a?: number, b?: number): void {}
}
    