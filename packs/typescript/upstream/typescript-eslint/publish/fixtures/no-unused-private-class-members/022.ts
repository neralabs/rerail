
class C {
  private privateMember;

  foo() {
    bar((this.privateMember += 1));
  }
}
    