
class B {
  dsw() {
    return function () {
      console.log(this);
      s(o => console.log(o, this));
    };
  }
}
      