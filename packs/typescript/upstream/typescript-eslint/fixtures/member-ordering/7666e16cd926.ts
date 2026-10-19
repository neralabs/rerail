
// no accessibility === public
class Foo {
  [A: string]: any;
  [a: string]: any;
  static C: boolean;
  static d: boolean;
  b: any;
  B: any;
  get e(): string {}
  get E(): string {}
  private imPrivate() {}
  private ImPrivate() {}
}
      