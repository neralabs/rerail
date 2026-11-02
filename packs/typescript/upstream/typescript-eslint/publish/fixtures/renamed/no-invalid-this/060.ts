
class A {
  b = new Array(1, 2, function () {
    console.log(this);
    s(o => console.log(o, this));
  });
}
      