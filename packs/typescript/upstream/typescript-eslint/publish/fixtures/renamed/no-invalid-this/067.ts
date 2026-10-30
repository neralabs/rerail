
Array.from([], function () {
  console.log(this);
  u(m => console.log(m, this));
});
      