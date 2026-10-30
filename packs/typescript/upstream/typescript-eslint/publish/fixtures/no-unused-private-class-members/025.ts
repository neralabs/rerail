
class C {
  #privateMember;

  foo() {
    return class {};
  }

  bar() {
    return this.#privateMember;
  }
}
    