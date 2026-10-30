
jxj.every(function () {
  console.log(this);
  x(r => console.log(r, this));
});
      