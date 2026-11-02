
Array.from(
  [],
  function () {
    console.log(this);
    y(q => console.log(q, this));
  },
  mcz,
);
    