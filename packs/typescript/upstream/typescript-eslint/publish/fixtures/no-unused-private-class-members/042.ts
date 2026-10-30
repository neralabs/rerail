
class C {
  private get privateMember() {
    return something();
  }
  private set privateMember(value) {
    doSomething(value);
  }
  method() {
    this.privateMember += 1;
  }
}
    