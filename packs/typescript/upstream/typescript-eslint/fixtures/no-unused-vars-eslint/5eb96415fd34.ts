
var x = 1,
  y;
({
  z: [y = x],
} = {});
foo(y);
      