
class C {
  private set privateMember(value) {
    doSomething(value);
  }

  foo() {
    this.privateMember = 1;
  }
}
    