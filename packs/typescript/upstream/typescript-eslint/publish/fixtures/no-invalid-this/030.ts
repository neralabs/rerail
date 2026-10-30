
Array.from(
  [],
  function () {
    console.log(this);
    z(x => console.log(x, this));
  },
  obj,
);
    