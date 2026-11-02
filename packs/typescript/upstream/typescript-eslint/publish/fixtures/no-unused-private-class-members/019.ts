
class Foo {
  #privateMember = 42;
  foo() {
    anotherMember = this.#privateMember;
  }
}
    