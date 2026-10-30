
class Foo {
  private privateMember;
  private privateMember2;

  method() {
    const { privateMember, privateMember2 } = this;
    console.log(privateMember, privateMember2);
  }
}
    