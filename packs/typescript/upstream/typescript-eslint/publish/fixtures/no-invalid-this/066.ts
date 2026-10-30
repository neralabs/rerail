
(function () {
  console.log(this);
  z(x => console.log(x, this));
}).apply(void 0);
      