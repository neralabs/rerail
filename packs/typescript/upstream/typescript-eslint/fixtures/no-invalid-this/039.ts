
foo(
  /* @this Obj */ function () {
    console.log(this);
    z(x => console.log(x, this));
  },
);
    