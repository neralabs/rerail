
class Foo {
  #privateMember;
  method() {
    [this.#privateMember = 1] = bar;
  }
}
      