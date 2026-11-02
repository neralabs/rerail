
class C {
  private privateMember;

  method() {
    ({ [this.privateMember]: a } = foo);
  }
}
    