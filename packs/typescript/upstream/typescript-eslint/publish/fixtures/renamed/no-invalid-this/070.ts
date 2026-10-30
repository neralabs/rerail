
fst.find(function () {
  console.log(this);
  t(q => console.log(q, this));
});
      