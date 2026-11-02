
class Foo {
  private prop: number;

  method() {
    const self1 = this;
    const self2 = self1;
    return self2.prop;
  }
}
      