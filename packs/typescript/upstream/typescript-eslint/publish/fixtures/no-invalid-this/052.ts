
var foo = function () {
  console.log(this);
  z(x => console.log(x, this));
}.bar(obj);
      