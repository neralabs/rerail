
class Foo {
  public static a: string;
  protected static b: string = '';
  private static c: string = '';

  @Dec() public d: string;
  @Dec() protected e: string;
  @Dec() private f: string;

  public g: string = '';
  protected h: string = '';
  private i: string = '';

  constructor() {}
}
      