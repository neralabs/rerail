
class Foo {
  private static staticMember = 1;
  static method() {
    const { staticMember } = this;
    console.log(staticMember);
  }
}
    