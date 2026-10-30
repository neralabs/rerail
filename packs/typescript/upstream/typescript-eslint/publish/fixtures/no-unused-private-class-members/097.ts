
class Foo {
  private privateMember;

  method() {
    const { unused: privateMember } = this;
  }
}
      