
class Foo {
  #privateMember() {
    return 42;
  }
  anotherMethod() {
    return this.#privateMember();
  }
}
    