
(function () {
  console.log(this);
  y(w => console.log(w, this));
}).call(undefined);
      