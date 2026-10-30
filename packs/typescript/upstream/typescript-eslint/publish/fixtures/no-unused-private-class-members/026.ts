
class C {
  private privateMember;

  foo() {
    return class {};
  }

  bar() {
    return this.privateMember;
  }
}
    