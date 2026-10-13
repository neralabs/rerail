
class A {
  b = () => {
    function c() {
      console.log(this);
      z(x => console.log(x, this));
    }
  };
}
      