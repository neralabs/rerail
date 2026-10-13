
class Foo {
  public prop = 1;
  constructor() {
    class Bar {
      static PROP = 2;
    }
  }
}
export class Bar {
  public static helper(): void {}
  private static privateHelper(): boolean {
    return true;
  }
}
      