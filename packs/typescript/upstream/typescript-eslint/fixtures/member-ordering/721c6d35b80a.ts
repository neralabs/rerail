
class Foo {
  public baz(): void;
  @Decorator() public baz() {}

  @Decorator() bar() {}
}
      