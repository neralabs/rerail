
class Foo {
  @Decorator() bar() {}

  public baz(): void;
  @Decorator() public baz() {}
}
      