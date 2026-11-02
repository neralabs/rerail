
class Foo {
  private privateMember = 1;

  method() {
    let privateMember;
    ({ privateMember } = this);
  }
}
    