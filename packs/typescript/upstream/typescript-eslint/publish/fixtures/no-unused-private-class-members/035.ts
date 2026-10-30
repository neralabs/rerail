
class C {
  #privateMember;

  method() {
    ({ [this.#privateMember]: a } = foo);
  }
}
    