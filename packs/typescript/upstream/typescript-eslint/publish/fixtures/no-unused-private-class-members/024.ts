
class Foo {
  private privateMember = 42;
  method() {
    return someGlobalMethod(this.privateMember);
  }
}
    