
class A {
  b = () => {
    function c() {
      console.log(this);
      u(q => console.log(q, this));
    }
  };
}
      