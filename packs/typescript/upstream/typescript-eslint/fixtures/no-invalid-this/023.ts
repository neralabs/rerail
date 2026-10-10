
var foo = function () {
  console.log(this);
  z(x => console.log(x, this));
}.bind(obj);
    