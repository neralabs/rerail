
class Foo {
  #privateMember;
  method() {
    [bar = 1] = this.#privateMember;
  }
}
    