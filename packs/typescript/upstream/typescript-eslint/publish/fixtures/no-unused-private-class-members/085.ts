
class Foo {
  #privateMember;
  method() {
    for (this.#privateMember in bar) {
    }
  }
}
      