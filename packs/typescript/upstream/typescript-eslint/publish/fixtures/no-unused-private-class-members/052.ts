
class C {
  set #privateMember(value) {
    doSomething(value);
  }

  foo() {
    this.#privateMember = 1;
  }
}
    