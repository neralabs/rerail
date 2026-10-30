
class C {
  get #privateMember() {
    return something();
  }
  set #privateMember(value) {
    doSomething(value);
  }
  method() {
    this.#privateMember += 1;
  }
}
    