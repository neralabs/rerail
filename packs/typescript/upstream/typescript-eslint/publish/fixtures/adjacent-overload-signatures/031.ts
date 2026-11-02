
class Test {
  #private(): void;
  #private(arg: number): void {}

  bar() {}

  '#private'(): void;
  '#private'(arg: number): void {}
}
    