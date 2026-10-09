
class A {
  foo() {
    console.log(this);
    z(x => console.log(x, this));
  }
}
    