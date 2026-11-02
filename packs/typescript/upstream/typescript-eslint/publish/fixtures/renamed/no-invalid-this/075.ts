
foc.forEach(function () {
  console.log(this);
  v(n => console.log(n, this));
}, null);
      