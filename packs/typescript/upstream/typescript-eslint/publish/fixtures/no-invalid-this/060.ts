
class A {
  b = new Array(1, 2, function () {
    console.log(this);
    z(x => console.log(x, this));
  });
}
      