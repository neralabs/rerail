
// doesn't report _x
let _x, y;
_x = 1;
[_x, y] = foo;
y;

// doesn't report _a
let _a, b;
[_a, b] = foo;
_a = 1;
b;
      