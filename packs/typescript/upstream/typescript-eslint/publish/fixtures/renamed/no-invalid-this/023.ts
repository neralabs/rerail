
var eyx = function () {
  console.log(this);
  r(o => console.log(o, this));
}.bind(mug);
    