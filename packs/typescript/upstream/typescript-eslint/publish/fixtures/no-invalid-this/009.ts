
class A {
  constructor() {
    console.log(this);
    z(x => console.log(x, this));
  }
}
      