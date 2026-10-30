
class Foo {
  private privateMember = 1;
  method() {
    const { privateMember: privateMember2 } = this;
  }
}
    