
class Foo {
  private privateMember;

  method() {
    const { privateMember: used } = this;
  }
}
    