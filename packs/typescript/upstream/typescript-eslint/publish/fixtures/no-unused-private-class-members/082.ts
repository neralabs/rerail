
class Foo {
  private privateMember() {}
  private privateMemberUsed() {
    return 42;
  }
  publicMethod() {
    return this.privateMemberUsed();
  }
}
      