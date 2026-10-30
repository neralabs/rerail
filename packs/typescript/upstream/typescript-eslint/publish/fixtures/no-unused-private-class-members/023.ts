
class Foo {
  #privateMember = 42;
  method() {
    return someGlobalMethod(this.#privateMember);
  }
}
    