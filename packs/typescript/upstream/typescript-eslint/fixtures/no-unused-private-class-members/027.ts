
class Foo {
  #privateMember;
  method() {
    for (const bar in this.#privateMember) {
    }
  }
}
    