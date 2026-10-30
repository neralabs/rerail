
fpj.findIndex(function () {
  console.log(this);
  t(n => console.log(n, this));
});
      