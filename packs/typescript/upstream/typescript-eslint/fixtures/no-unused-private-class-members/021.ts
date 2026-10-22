
class C {
  #privateMember;

  foo() {
    bar((this.#privateMember += 1));
  }
}
    