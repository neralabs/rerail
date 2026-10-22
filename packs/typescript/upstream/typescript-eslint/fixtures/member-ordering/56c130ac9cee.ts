
abstract class Foo {
  protected typeChecker: (data: any) => boolean;
  public abstract required: boolean;
  abstract verify(): void;
}
      