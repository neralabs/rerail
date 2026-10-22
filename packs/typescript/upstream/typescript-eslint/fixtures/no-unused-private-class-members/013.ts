
class Foo {
  private static staticProp: number;

  method(thing: typeof Foo) {
    return thing.staticProp;
  }
}
    