
class Foo {
  #privateMember() {}
  #privateMemberUsed() {
    return 42;
  }
  publicMethod() {
    return this.#privateMemberUsed();
  }
}
      