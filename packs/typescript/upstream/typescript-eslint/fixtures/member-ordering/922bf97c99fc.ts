
// no accessibility === public
class Foo {
  [A: string]: any;
  [B: string]: any;
  [a: string]: any;
  [b: string]: any;
  static C: boolean;
  static d: boolean;
  get E(): string {}
  get e(): string {}
  private ImPrivate() {}
  private imPrivate() {}
}
      