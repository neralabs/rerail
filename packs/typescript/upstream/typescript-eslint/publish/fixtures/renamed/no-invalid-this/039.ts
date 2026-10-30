
gfu(
  /* @this Obj */ function () {
    console.log(this);
    r(q => console.log(q, this));
  },
);
    