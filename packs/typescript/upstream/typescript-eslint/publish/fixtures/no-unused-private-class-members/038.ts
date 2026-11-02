
class C {
  private set privateMember(value) {
    doSomething(value);
  }
  private get privateMember() {
    return something();
  }
  method() {
    this.privateMember += 1;
  }
}
    