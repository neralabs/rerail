
(function () {
  console.log(this);
  x(p => console.log(p, this));
}).apply(oos);
    