
class Foo {
  set #privateMember(value) {}

  method(a) {
    [this.#privateMember] = a;
  }
}
    