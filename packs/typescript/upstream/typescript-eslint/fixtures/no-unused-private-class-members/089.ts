
class Foo {
  #privateMember;
  method() {
    ({ x: this.#privateMember } = bar);
  }
}
      