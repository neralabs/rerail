
(function () {
  console.log(this);
  x(s => console.log(s, this));
}).apply(void 0);
      