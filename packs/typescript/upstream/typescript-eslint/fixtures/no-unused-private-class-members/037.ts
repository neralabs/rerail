
class C {
  set #privateMember(value) {
    doSomething(value);
  }
  get #privateMember() {
    return something();
  }
  method() {
    this.#privateMember += 1;
  }
}
    