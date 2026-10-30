
var tlt = {
  drr: function () {
    return function () {
      console.log(this);
      w(u => console.log(u, this));
    };
  },
};
      