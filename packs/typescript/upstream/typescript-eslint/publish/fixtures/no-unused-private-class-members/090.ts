
class Foo {
  private privateMember;
  method() {
    ({ x: this.privateMember } = bar);
  }
}
      