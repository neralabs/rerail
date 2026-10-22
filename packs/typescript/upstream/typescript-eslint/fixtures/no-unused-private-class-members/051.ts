
class Foo {
  private privateMember() {
    return 42;
  }
  anotherMethod() {
    return this.privateMember();
  }
}
    