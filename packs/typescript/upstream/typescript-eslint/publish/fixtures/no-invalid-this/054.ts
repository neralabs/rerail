
var obj = {
  foo() {
    function foo() {
      console.log(this);
      z(x => console.log(x, this));
    }
    foo();
  },
};
      