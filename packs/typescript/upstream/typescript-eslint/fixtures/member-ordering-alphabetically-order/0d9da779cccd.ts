
class Foo {
  @Dec() a1: string;
  @Dec()
  a3: string;
  @Dec()
  a2: string;

  constructor() {}

  b1: string;
  b2: string;

  public c(): void;
  @Dec() d(): void {}
}
      