
(function () {
  console.log(this);
  s(n => console.log(n, this));
}).call(mwh);
    