
function foo(
  func = function () {
    console.log(this);
    z(x => console.log(x, this));
  },
) {}
      