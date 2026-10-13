
class A {
  static foo() {
    console.log(this);
    z(x => console.log(x, this));
  }
}
    