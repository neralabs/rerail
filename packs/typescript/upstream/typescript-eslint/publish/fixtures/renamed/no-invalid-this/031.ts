
ggf.every(function () {
  console.log(this);
  q(o => console.log(o, this));
}, mpy);
    