
var obj = {
  foo: function () {
    function foo() {
      console.log(this);
      z(x => console.log(x, this));
    }
    foo();
  },
};
      