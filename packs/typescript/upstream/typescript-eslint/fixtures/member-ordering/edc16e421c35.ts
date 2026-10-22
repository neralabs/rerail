
class Foo {
  public bar(): void;
  @Decorator() bar() {}

  public baz(): void;
  @Decorator() public baz() {}
}
      