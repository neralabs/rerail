
class Foo {
  private set privateMember(value) {}

  method(a) {
    [this.privateMember] = a;
  }
}
    