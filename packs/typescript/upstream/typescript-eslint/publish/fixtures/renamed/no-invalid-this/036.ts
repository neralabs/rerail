
jlp.map(function () {
  console.log(this);
  x(v => console.log(v, this));
}, sfy);
    