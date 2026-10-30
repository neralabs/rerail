
var njk = {
  hkz: (function () {
    return function () {
      console.log(this);
      r(o => console.log(o, this));
    };
  })(),
};
      