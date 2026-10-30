
Reflect.apply(
  function () {
    console.log(this);
    w(u => console.log(u, this));
  },
  nfw,
  [],
);
    