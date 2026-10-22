
class Foo {
  #privateMember;
  method() {
    [...this.#privateMember] = bar;
  }
}
      