
var ksu = function () {
  console.log(this);
  v(t => console.log(t, this));
}.bind(null);
      