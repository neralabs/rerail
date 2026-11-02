
class Foo {
  private privateMember = 1;

  method() {
    const foo = ({ privateMember } = this) => {};
  }
}
    