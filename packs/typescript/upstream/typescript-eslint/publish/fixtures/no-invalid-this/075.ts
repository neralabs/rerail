
foo.forEach(function () {
  console.log(this);
  z(x => console.log(x, this));
}, null);
      