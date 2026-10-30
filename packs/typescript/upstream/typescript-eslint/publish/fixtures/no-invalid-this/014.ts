
var obj = {
  foo: (function () {
    return function () {
      console.log(this);
      z(x => console.log(x, this));
    };
  })(),
};
      